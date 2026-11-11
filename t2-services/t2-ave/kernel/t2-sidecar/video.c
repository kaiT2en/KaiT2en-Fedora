#define pr_fmt(fmt) KBUILD_MODNAME ": " fmt

#include "sidecar.h"

#include <linux/module.h>
#include <linux/slab.h>
#include <linux/usb.h>
#include <linux/workqueue.h>
#include <media/v4l2-ctrls.h>
#include <media/v4l2-device.h>
#include <media/v4l2-event.h>
#include <media/v4l2-ioctl.h>
#include <media/v4l2-mem2mem.h>
#include <media/videobuf2-vmalloc.h>

#define T2SC_NAME		"t2-sidecar"
#define T2SC_MIN_WIDTH		128
#define T2SC_MIN_HEIGHT		128
#define T2SC_MAX_WIDTH		4096
#define T2SC_MAX_HEIGHT		4096
#define T2SC_DEFAULT_WIDTH	1920
#define T2SC_DEFAULT_HEIGHT	1080
#define T2SC_DEFAULT_FPS	60
#define T2SC_DEFAULT_BITRATE	10000000
#define T2SC_MIN_BITRATE	100000
#define T2SC_MAX_BITRATE	100000000
#define T2SC_MAX_GOP		32767
#define T2SC_META_SLOTS		64
/* Encoded frames waiting for a CAPTURE buffer before the oldest is dropped */
#define T2SC_MAX_PENDING	16
/* OUTPUT buffers in the ready queue, the one on the T2 included */
#define T2SC_MAX_WAITING	2
#define T2SC_QBUF_WAIT_MS	1000

#define T2SC_STRIDE(w)		ALIGN((w), 64)
#define T2SC_NV12_SIZE(w, h)	(T2SC_STRIDE(w) * ((h) + DIV_ROUND_UP((h), 2)))
#define T2SC_CAP_SIZE(w, h)	max_t(u32, (w) * (h) * 3 / 2, SZ_512K)

struct t2sc_video {
	struct t2sc_device *dev;
	struct v4l2_device v4l2_dev;
	struct video_device vdev;
	struct v4l2_m2m_dev *m2m_dev;
	struct workqueue_struct *wq;
	/* serialises ioctls and both vb2 queues */
	struct mutex lock;
	/* the one context with an open T2 session */
	struct t2sc_ctx *active;
};

/* OUTPUT buffer, carrying a keyframe request made just before it was queued */
struct t2sc_buffer {
	struct v4l2_m2m_buffer m2m;
	bool force_keyframe;
};

struct t2sc_meta {
	u64 timestamp;
	struct v4l2_timecode timecode;
	u32 flags;
	u32 sequence;
};

struct t2sc_ctx {
	struct v4l2_fh fh;
	struct t2sc_video *video;
	struct v4l2_pix_format_mplane src_fmt;
	struct v4l2_pix_format_mplane dst_fmt;
	struct v4l2_ctrl_handler ctrls;
	struct work_struct run_work;
	struct work_struct drain_work;
	/* woken whenever an OUTPUT buffer leaves the ready queue */
	wait_queue_head_t src_wait;

	u32 fps;
	u32 bitrate;
	u32 gop_size;
	bool bitrate_dirty;
	/* set by the control, moved onto the next queued OUTPUT buffer */
	bool force_keyframe;

	struct t2sc_session *session;
	u32 next_frame_id;
	struct t2sc_meta meta[T2SC_META_SLOTS];

	/* taken after frame_lock or the queue lock, never before */
	struct mutex out_lock;
	struct list_head out_list;
	unsigned int out_count;
	bool drain_done;
};

static const struct v4l2_event t2sc_eos_event = {
	.type = V4L2_EVENT_EOS,
};

static struct t2sc_ctx *fh_to_ctx(struct file *file)
{
	return container_of(file->private_data, struct t2sc_ctx, fh);
}

static void t2sc_fill_src_fmt(struct v4l2_pix_format_mplane *pix, u32 w, u32 h)
{
	w = ALIGN(clamp_t(u32, w, T2SC_MIN_WIDTH, T2SC_MAX_WIDTH), 2);
	h = ALIGN(clamp_t(u32, h, T2SC_MIN_HEIGHT, T2SC_MAX_HEIGHT), 2);
	pix->width = w;
	pix->height = h;
	pix->pixelformat = V4L2_PIX_FMT_NV12;
	pix->field = V4L2_FIELD_NONE;
	pix->num_planes = 1;
	pix->plane_fmt[0].bytesperline = T2SC_STRIDE(w);
	pix->plane_fmt[0].sizeimage = T2SC_NV12_SIZE(w, h);
	if (!pix->colorspace)
		pix->colorspace = V4L2_COLORSPACE_REC709;
}

static void t2sc_fill_dst_fmt(struct v4l2_pix_format_mplane *pix, u32 w, u32 h, u32 codec)
{
	pix->width = w;
	pix->height = h;
	pix->pixelformat = codec == V4L2_PIX_FMT_H264 ? V4L2_PIX_FMT_H264 : V4L2_PIX_FMT_HEVC;
	pix->field = V4L2_FIELD_NONE;
	pix->num_planes = 1;
	pix->plane_fmt[0].bytesperline = 0;
	pix->plane_fmt[0].sizeimage = T2SC_CAP_SIZE(w, h);
}

static int t2sc_querycap(struct file *file, void *priv, struct v4l2_capability *cap)
{
	struct t2sc_video *video = video_drvdata(file);

	strscpy(cap->driver, "t2-sidecar", sizeof(cap->driver));
	strscpy(cap->card, "Apple T2 Sidecar Encoder", sizeof(cap->card));
	usb_make_path(video->dev->udev, cap->bus_info, sizeof(cap->bus_info));
	return 0;
}

static int t2sc_enum_fmt(struct file *file, void *priv, struct v4l2_fmtdesc *f)
{
	static const u32 coded[] = { V4L2_PIX_FMT_H264, V4L2_PIX_FMT_HEVC };

	if (V4L2_TYPE_IS_OUTPUT(f->type)) {
		if (f->index)
			return -EINVAL;
		f->pixelformat = V4L2_PIX_FMT_NV12;
		return 0;
	}
	if (f->index >= ARRAY_SIZE(coded))
		return -EINVAL;
	f->pixelformat = coded[f->index];
	f->flags = V4L2_FMT_FLAG_COMPRESSED;
	return 0;
}

static int t2sc_g_fmt(struct file *file, void *priv, struct v4l2_format *f)
{
	struct t2sc_ctx *ctx = fh_to_ctx(file);

	f->fmt.pix_mp = V4L2_TYPE_IS_OUTPUT(f->type) ? ctx->src_fmt : ctx->dst_fmt;
	return 0;
}

static int t2sc_try_fmt_out(struct file *file, void *priv, struct v4l2_format *f)
{
	t2sc_fill_src_fmt(&f->fmt.pix_mp, f->fmt.pix_mp.width, f->fmt.pix_mp.height);
	return 0;
}

static int t2sc_try_fmt_cap(struct file *file, void *priv, struct v4l2_format *f)
{
	struct t2sc_ctx *ctx = fh_to_ctx(file);
	struct v4l2_pix_format_mplane *pix = &f->fmt.pix_mp;

	t2sc_fill_dst_fmt(pix, ctx->src_fmt.width, ctx->src_fmt.height, pix->pixelformat);
	return 0;
}

static int t2sc_s_fmt_out(struct file *file, void *priv, struct v4l2_format *f)
{
	struct t2sc_ctx *ctx = fh_to_ctx(file);

	if (vb2_is_busy(v4l2_m2m_get_src_vq(ctx->fh.m2m_ctx)))
		return -EBUSY;
	t2sc_try_fmt_out(file, priv, f);
	ctx->src_fmt = f->fmt.pix_mp;
	t2sc_fill_dst_fmt(&ctx->dst_fmt, ctx->src_fmt.width, ctx->src_fmt.height,
			  ctx->dst_fmt.pixelformat);
	ctx->dst_fmt.colorspace = ctx->src_fmt.colorspace;
	return 0;
}

static int t2sc_s_fmt_cap(struct file *file, void *priv, struct v4l2_format *f)
{
	struct t2sc_ctx *ctx = fh_to_ctx(file);

	if (vb2_is_busy(v4l2_m2m_get_dst_vq(ctx->fh.m2m_ctx)))
		return -EBUSY;
	t2sc_try_fmt_cap(file, priv, f);
	ctx->dst_fmt = f->fmt.pix_mp;
	return 0;
}

static int t2sc_enum_framesizes(struct file *file, void *priv, struct v4l2_frmsizeenum *fs)
{
	if (fs->index || (fs->pixel_format != V4L2_PIX_FMT_NV12 &&
			  fs->pixel_format != V4L2_PIX_FMT_H264 &&
			  fs->pixel_format != V4L2_PIX_FMT_HEVC))
		return -EINVAL;
	fs->type = V4L2_FRMSIZE_TYPE_STEPWISE;
	fs->stepwise.min_width = T2SC_MIN_WIDTH;
	fs->stepwise.max_width = T2SC_MAX_WIDTH;
	fs->stepwise.step_width = 2;
	fs->stepwise.min_height = T2SC_MIN_HEIGHT;
	fs->stepwise.max_height = T2SC_MAX_HEIGHT;
	fs->stepwise.step_height = 2;
	return 0;
}

static int t2sc_g_parm(struct file *file, void *priv, struct v4l2_streamparm *sp)
{
	struct t2sc_ctx *ctx = fh_to_ctx(file);
	struct v4l2_fract *tpf;

	memset(&sp->parm, 0, sizeof(sp->parm));
	if (V4L2_TYPE_IS_OUTPUT(sp->type)) {
		sp->parm.output.capability = V4L2_CAP_TIMEPERFRAME;
		tpf = &sp->parm.output.timeperframe;
	} else {
		sp->parm.capture.capability = V4L2_CAP_TIMEPERFRAME;
		tpf = &sp->parm.capture.timeperframe;
	}
	tpf->numerator = 1;
	tpf->denominator = ctx->fps;
	return 0;
}

static int t2sc_s_parm(struct file *file, void *priv, struct v4l2_streamparm *sp)
{
	struct t2sc_ctx *ctx = fh_to_ctx(file);
	struct v4l2_fract *tpf = V4L2_TYPE_IS_OUTPUT(sp->type) ?
		&sp->parm.output.timeperframe : &sp->parm.capture.timeperframe;

	if (tpf->numerator && tpf->denominator)
		ctx->fps = clamp_t(u32, DIV_ROUND_CLOSEST(tpf->denominator, tpf->numerator), 1, 240);
	return t2sc_g_parm(file, priv, sp);
}

static int t2sc_encoder_cmd(struct file *file, void *priv, struct v4l2_encoder_cmd *ec)
{
	struct t2sc_ctx *ctx = fh_to_ctx(file);
	struct v4l2_m2m_ctx *m2m = ctx->fh.m2m_ctx;
	int ret;

	ret = v4l2_m2m_ioctl_try_encoder_cmd(file, priv, ec);
	if (ret)
		return ret;
	/*
	 * Input buffers return once the T2 has taken their frame, so with none
	 * queued the last frame can still be on the T2. The framework would flag a
	 * CAPTURE buffer LAST right away, ahead of them. Drain through
	 * CompleteFrames instead. With an input buffer queued, its job does it.
	 */
	if (ec->cmd == V4L2_ENC_CMD_STOP && ctx->session && !m2m->is_draining &&
	    !m2m->has_stopped && !v4l2_m2m_num_src_bufs_ready(m2m)) {
		m2m->is_draining = true;
		queue_work(ctx->video->wq, &ctx->drain_work);
		return 0;
	}
	return v4l2_m2m_ioctl_encoder_cmd(file, priv, ec);
}

static int t2sc_qbuf(struct file *file, void *priv, struct v4l2_buffer *buf)
{
	struct t2sc_ctx *ctx = fh_to_ctx(file);
	struct v4l2_m2m_ctx *m2m = ctx->fh.m2m_ctx;

	if (V4L2_TYPE_IS_OUTPUT(buf->type) && ctx->session &&
	    v4l2_m2m_num_src_bufs_ready(m2m) >= T2SC_MAX_WAITING) {
		long ret;

		mutex_unlock(&ctx->video->lock);
		ret = wait_event_interruptible_timeout(ctx->src_wait,
				v4l2_m2m_num_src_bufs_ready(m2m) < T2SC_MAX_WAITING ||
				!READ_ONCE(ctx->session),
				msecs_to_jiffies(T2SC_QBUF_WAIT_MS));
		mutex_lock(&ctx->video->lock);
		if (ret < 0)
			return ret;
	}
	return v4l2_m2m_ioctl_qbuf(file, priv, buf);
}

static int t2sc_subscribe_event(struct v4l2_fh *fh, const struct v4l2_event_subscription *sub)
{
	if (sub->type == V4L2_EVENT_EOS)
		return v4l2_event_subscribe(fh, sub, 0, NULL);
	return v4l2_ctrl_subscribe_event(fh, sub);
}

static const struct v4l2_ioctl_ops t2sc_ioctl_ops = {
	.vidioc_querycap		= t2sc_querycap,
	.vidioc_enum_fmt_vid_cap	= t2sc_enum_fmt,
	.vidioc_enum_fmt_vid_out	= t2sc_enum_fmt,
	.vidioc_g_fmt_vid_cap_mplane	= t2sc_g_fmt,
	.vidioc_g_fmt_vid_out_mplane	= t2sc_g_fmt,
	.vidioc_try_fmt_vid_cap_mplane	= t2sc_try_fmt_cap,
	.vidioc_try_fmt_vid_out_mplane	= t2sc_try_fmt_out,
	.vidioc_s_fmt_vid_cap_mplane	= t2sc_s_fmt_cap,
	.vidioc_s_fmt_vid_out_mplane	= t2sc_s_fmt_out,
	.vidioc_enum_framesizes		= t2sc_enum_framesizes,
	.vidioc_g_parm			= t2sc_g_parm,
	.vidioc_s_parm			= t2sc_s_parm,
	.vidioc_reqbufs			= v4l2_m2m_ioctl_reqbufs,
	.vidioc_querybuf		= v4l2_m2m_ioctl_querybuf,
	.vidioc_qbuf			= t2sc_qbuf,
	.vidioc_dqbuf			= v4l2_m2m_ioctl_dqbuf,
	.vidioc_create_bufs		= v4l2_m2m_ioctl_create_bufs,
	.vidioc_prepare_buf		= v4l2_m2m_ioctl_prepare_buf,
	.vidioc_expbuf			= v4l2_m2m_ioctl_expbuf,
	.vidioc_streamon		= v4l2_m2m_ioctl_streamon,
	.vidioc_streamoff		= v4l2_m2m_ioctl_streamoff,
	.vidioc_encoder_cmd		= t2sc_encoder_cmd,
	.vidioc_try_encoder_cmd		= v4l2_m2m_ioctl_try_encoder_cmd,
	.vidioc_subscribe_event		= t2sc_subscribe_event,
	.vidioc_unsubscribe_event	= v4l2_event_unsubscribe,
};

/*
 * Encoded output arrives from the receive thread, independent of the job
 * that sent the frame, and goes to whichever CAPTURE buffer is queued.
 */
static void t2sc_signal_eos(struct t2sc_ctx *ctx)
{
	struct vb2_v4l2_buffer *dst = v4l2_m2m_dst_buf_remove(ctx->fh.m2m_ctx);

	if (!dst)
		return;
	vb2_set_plane_payload(&dst->vb2_buf, 0, 0);
	dst->flags |= V4L2_BUF_FLAG_LAST;
	v4l2_m2m_buf_done(dst, VB2_BUF_STATE_DONE);
	ctx->drain_done = false;
	v4l2_m2m_mark_stopped(ctx->fh.m2m_ctx);
	v4l2_event_queue_fh(&ctx->fh, &t2sc_eos_event);
}

static void t2sc_flush_output(struct t2sc_ctx *ctx)
{
	mutex_lock(&ctx->out_lock);
	while (!list_empty(&ctx->out_list)) {
		struct t2sc_frame *f = list_first_entry(&ctx->out_list, struct t2sc_frame, node);
		const struct t2sc_meta *m = &ctx->meta[f->frame_id % T2SC_META_SLOTS];
		struct vb2_v4l2_buffer *dst;

		dst = v4l2_m2m_dst_buf_remove(ctx->fh.m2m_ctx);
		if (!dst)
			break;
		list_del(&f->node);
		ctx->out_count--;

		if (f->len > vb2_plane_size(&dst->vb2_buf, 0)) {
			pr_warn("frame %u: %zu bytes exceed the CAPTURE buffer\n", f->frame_id, f->len);
			vb2_set_plane_payload(&dst->vb2_buf, 0, 0);
			v4l2_m2m_buf_done(dst, VB2_BUF_STATE_ERROR);
			kvfree(f);
			continue;
		}
		memcpy(vb2_plane_vaddr(&dst->vb2_buf, 0), f->data, f->len);
		vb2_set_plane_payload(&dst->vb2_buf, 0, f->len);
		dst->vb2_buf.timestamp = m->timestamp;
		dst->timecode = m->timecode;
		dst->sequence = m->sequence;
		dst->flags &= ~(V4L2_BUF_FLAG_KEYFRAME | V4L2_BUF_FLAG_PFRAME |
				V4L2_BUF_FLAG_BFRAME | V4L2_BUF_FLAG_TIMECODE);
		dst->flags |= m->flags & V4L2_BUF_FLAG_TIMECODE;
		dst->flags |= f->keyframe ? V4L2_BUF_FLAG_KEYFRAME : V4L2_BUF_FLAG_PFRAME;
		if (ctx->drain_done && list_empty(&ctx->out_list)) {
			dst->flags |= V4L2_BUF_FLAG_LAST;
			ctx->drain_done = false;
			v4l2_m2m_mark_stopped(ctx->fh.m2m_ctx);
			v4l2_event_queue_fh(&ctx->fh, &t2sc_eos_event);
		}
		v4l2_m2m_buf_done(dst, VB2_BUF_STATE_DONE);
		kvfree(f);
	}
	if (ctx->drain_done && list_empty(&ctx->out_list))
		t2sc_signal_eos(ctx);
	mutex_unlock(&ctx->out_lock);
}

/* Lets the T2 emit everything, then flags the final CAPTURE buffer LAST. */
static void t2sc_finish_drain(struct t2sc_ctx *ctx)
{
	if (ctx->session)
		t2sc_session_complete(ctx->session);
	mutex_lock(&ctx->out_lock);
	ctx->drain_done = true;
	mutex_unlock(&ctx->out_lock);
	t2sc_flush_output(ctx);
}

static void t2sc_drain_work(struct work_struct *work)
{
	t2sc_finish_drain(container_of(work, struct t2sc_ctx, drain_work));
}

/* Called by the receive thread with dev->frame_lock held. */
static void t2sc_on_frame(struct t2sc_video *video, struct t2sc_frame *f)
{
	struct t2sc_ctx *ctx = READ_ONCE(video->active);

	if (!ctx || f->status || !f->len) {
		if (f->status)
			pr_warn("frame %u: encoder status %d\n", f->frame_id, f->status);
		kvfree(f);
		return;
	}
	mutex_lock(&ctx->out_lock);
	if (ctx->out_count >= T2SC_MAX_PENDING) {
		struct t2sc_frame *old = list_first_entry(&ctx->out_list, struct t2sc_frame, node);

		pr_warn_ratelimited("no CAPTURE buffer queued, dropping frame %u\n", old->frame_id);
		list_del(&old->node);
		kvfree(old);
		ctx->out_count--;
	}
	list_add_tail(&f->node, &ctx->out_list);
	ctx->out_count++;
	mutex_unlock(&ctx->out_lock);
	t2sc_flush_output(ctx);
}

static void t2sc_drop_output(struct t2sc_ctx *ctx)
{
	struct t2sc_frame *f, *tmp;

	mutex_lock(&ctx->out_lock);
	list_for_each_entry_safe(f, tmp, &ctx->out_list, node) {
		list_del(&f->node);
		kvfree(f);
	}
	ctx->out_count = 0;
	ctx->drain_done = false;
	mutex_unlock(&ctx->out_lock);
}

static int t2sc_queue_setup(struct vb2_queue *vq, unsigned int *nbuffers, unsigned int *nplanes,
			    unsigned int sizes[], struct device *alloc_devs[])
{
	struct t2sc_ctx *ctx = vb2_get_drv_priv(vq);
	struct v4l2_pix_format_mplane *fmt = V4L2_TYPE_IS_OUTPUT(vq->type) ?
		&ctx->src_fmt : &ctx->dst_fmt;

	if (*nplanes)
		return *nplanes != 1 || sizes[0] < fmt->plane_fmt[0].sizeimage ? -EINVAL : 0;
	*nplanes = 1;
	sizes[0] = fmt->plane_fmt[0].sizeimage;
	return 0;
}

static int t2sc_buf_prepare(struct vb2_buffer *vb)
{
	struct t2sc_ctx *ctx = vb2_get_drv_priv(vb->vb2_queue);
	struct v4l2_pix_format_mplane *fmt = V4L2_TYPE_IS_OUTPUT(vb->type) ?
		&ctx->src_fmt : &ctx->dst_fmt;

	if (vb2_plane_size(vb, 0) < fmt->plane_fmt[0].sizeimage)
		return -EINVAL;
	if (V4L2_TYPE_IS_OUTPUT(vb->type))
		vb2_set_plane_payload(vb, 0, fmt->plane_fmt[0].sizeimage);
	return 0;
}

static void t2sc_buf_queue(struct vb2_buffer *vb)
{
	struct t2sc_ctx *ctx = vb2_get_drv_priv(vb->vb2_queue);
	struct vb2_v4l2_buffer *vbuf = to_vb2_v4l2_buffer(vb);

	/*
	 * FFmpeg sets FORCE_KEY_FRAME right before queueing the frame it means,
	 * while older frames may still wait ahead of it.
	 */
	if (V4L2_TYPE_IS_OUTPUT(vb->type))
		container_of(vbuf, struct t2sc_buffer, m2m.vb)->force_keyframe =
			xchg(&ctx->force_keyframe, false);
	v4l2_m2m_buf_queue(ctx->fh.m2m_ctx, vbuf);
	if (!V4L2_TYPE_IS_OUTPUT(vb->type))
		t2sc_flush_output(ctx);
}

static void t2sc_return_bufs(struct t2sc_ctx *ctx, bool output, enum vb2_buffer_state state)
{
	struct vb2_v4l2_buffer *vbuf;

	while ((vbuf = output ? v4l2_m2m_src_buf_remove(ctx->fh.m2m_ctx) :
				v4l2_m2m_dst_buf_remove(ctx->fh.m2m_ctx)))
		v4l2_m2m_buf_done(vbuf, state);
}

static int t2sc_start_streaming(struct vb2_queue *vq, unsigned int count)
{
	struct t2sc_ctx *ctx = vb2_get_drv_priv(vq);
	struct t2sc_video *video = ctx->video;
	struct t2sc_session_params p;
	int ret;

	if (!V4L2_TYPE_IS_OUTPUT(vq->type)) {
		v4l2_m2m_update_start_streaming_state(ctx->fh.m2m_ctx, vq);
		return 0;
	}
	if (video->active) {
		ret = -EBUSY;
		goto err;
	}
	p = (struct t2sc_session_params) {
		.codec = ctx->dst_fmt.pixelformat == V4L2_PIX_FMT_H264 ?
			 T2SC_CODEC_H264 : T2SC_CODEC_HEVC,
		.width = ctx->src_fmt.width,
		.height = ctx->src_fmt.height,
		.fps = ctx->fps,
		.bitrate = ctx->bitrate,
		.gop_size = ctx->gop_size,
	};
	ret = t2sc_session_open(video->dev, &p, &ctx->session);
	if (ret)
		goto err;
	ctx->bitrate_dirty = false;
	ctx->next_frame_id = 0;
	ctx->drain_done = false;
	WRITE_ONCE(video->active, ctx);
	v4l2_m2m_update_start_streaming_state(ctx->fh.m2m_ctx, vq);
	return 0;

err:
	t2sc_return_bufs(ctx, true, VB2_BUF_STATE_QUEUED);
	return ret;
}

static void t2sc_stop_streaming(struct vb2_queue *vq)
{
	struct t2sc_ctx *ctx = vb2_get_drv_priv(vq);
	struct t2sc_video *video = ctx->video;

	v4l2_m2m_update_stop_streaming_state(ctx->fh.m2m_ctx, vq);
	if (V4L2_TYPE_IS_OUTPUT(vq->type)) {
		flush_work(&ctx->run_work);
		flush_work(&ctx->drain_work);
		if (ctx->session) {
			t2sc_session_close(ctx->session);
			WRITE_ONCE(ctx->session, NULL);
			wake_up(&ctx->src_wait);
		}
		if (video->active == ctx)
			WRITE_ONCE(video->active, NULL);
		t2sc_drop_output(ctx);
		t2sc_return_bufs(ctx, true, VB2_BUF_STATE_ERROR);
	} else {
		t2sc_return_bufs(ctx, false, VB2_BUF_STATE_ERROR);
	}
}

static const struct vb2_ops t2sc_vb2_ops = {
	.queue_setup		= t2sc_queue_setup,
	.buf_prepare		= t2sc_buf_prepare,
	.buf_queue		= t2sc_buf_queue,
	.start_streaming	= t2sc_start_streaming,
	.stop_streaming		= t2sc_stop_streaming,
};

static void t2sc_run_work(struct work_struct *work)
{
	struct t2sc_ctx *ctx = container_of(work, struct t2sc_ctx, run_work);
	struct t2sc_video *video = ctx->video;
	struct vb2_v4l2_buffer *src;
	struct t2sc_meta *m;
	bool last;
	u32 id;
	u8 *y;
	int ret;

	src = v4l2_m2m_next_src_buf(ctx->fh.m2m_ctx);
	if (!src)
		goto finish;
	if (!ctx->session) {
		last = v4l2_m2m_is_last_draining_src_buf(ctx->fh.m2m_ctx, src);
		v4l2_m2m_src_buf_remove(ctx->fh.m2m_ctx);
		v4l2_m2m_buf_done(src, VB2_BUF_STATE_ERROR);
		wake_up(&ctx->src_wait);
		goto drain;
	}

	if (READ_ONCE(ctx->bitrate_dirty)) {
		WRITE_ONCE(ctx->bitrate_dirty, false);
		t2sc_session_set_bitrate(ctx->session, ctx->bitrate);
	}

	id = ctx->next_frame_id++;
	m = &ctx->meta[id % T2SC_META_SLOTS];
	m->timestamp = src->vb2_buf.timestamp;
	m->timecode = src->timecode;
	m->flags = src->flags;
	m->sequence = src->sequence;

	y = vb2_plane_vaddr(&src->vb2_buf, 0);
	t2sc_session_prepare(ctx->session, id, div_u64(src->vb2_buf.timestamp, NSEC_PER_USEC),
			     ctx->fps, y, ctx->src_fmt.plane_fmt[0].bytesperline,
			     y + ctx->src_fmt.plane_fmt[0].bytesperline * ctx->src_fmt.height,
			     ctx->src_fmt.plane_fmt[0].bytesperline,
			     container_of(src, struct t2sc_buffer, m2m.vb)->force_keyframe);
	ret = t2sc_session_submit(ctx->session);
	if (ret)
		pr_warn_ratelimited("frame %u not taken by the T2 (%d)\n", id, ret);
	/* A suspend closed the session, the client has to open a new one */
	if (ret == -ENODEV)
		vb2_queue_error(v4l2_m2m_get_dst_vq(ctx->fh.m2m_ctx));

	v4l2_m2m_src_buf_remove(ctx->fh.m2m_ctx);
	last = v4l2_m2m_is_last_draining_src_buf(ctx->fh.m2m_ctx, src);
	v4l2_m2m_buf_done(src, ret == -ENODEV ? VB2_BUF_STATE_ERROR : VB2_BUF_STATE_DONE);
	wake_up(&ctx->src_wait);

drain:
	if (last)
		t2sc_finish_drain(ctx);
finish:
	v4l2_m2m_job_finish(video->m2m_dev, ctx->fh.m2m_ctx);
}

/* Runs in the caller's context, often QBUF with the queue lock held. */
static void t2sc_device_run(void *priv)
{
	struct t2sc_ctx *ctx = priv;

	queue_work(ctx->video->wq, &ctx->run_work);
}

static const struct v4l2_m2m_ops t2sc_m2m_ops = {
	.device_run = t2sc_device_run,
};

static int t2sc_s_ctrl(struct v4l2_ctrl *ctrl)
{
	struct t2sc_ctx *ctx = container_of(ctrl->handler, struct t2sc_ctx, ctrls);

	switch (ctrl->id) {
	case V4L2_CID_MPEG_VIDEO_BITRATE:
		ctx->bitrate = ctrl->val;
		WRITE_ONCE(ctx->bitrate_dirty, true);
		break;
	case V4L2_CID_MPEG_VIDEO_FORCE_KEY_FRAME:
		ctx->force_keyframe = true;
		break;
	case V4L2_CID_MPEG_VIDEO_GOP_SIZE:
		ctx->gop_size = ctrl->val;
		break;
	default:
		/* Accepted, the T2 chooses profile, level and rate control itself. */
		break;
	}
	return 0;
}

static const struct v4l2_ctrl_ops t2sc_ctrl_ops = {
	.s_ctrl = t2sc_s_ctrl,
};

static int t2sc_init_ctrls(struct t2sc_ctx *ctx)
{
	struct v4l2_ctrl_handler *hdl = &ctx->ctrls;

	v4l2_ctrl_handler_init(hdl, 8);
	v4l2_ctrl_new_std(hdl, &t2sc_ctrl_ops, V4L2_CID_MPEG_VIDEO_BITRATE,
			  T2SC_MIN_BITRATE, T2SC_MAX_BITRATE, 1, T2SC_DEFAULT_BITRATE);
	v4l2_ctrl_new_std(hdl, &t2sc_ctrl_ops, V4L2_CID_MPEG_VIDEO_FORCE_KEY_FRAME, 0, 0, 0, 0);
	v4l2_ctrl_new_std(hdl, &t2sc_ctrl_ops, V4L2_CID_MPEG_VIDEO_GOP_SIZE,
			  0, T2SC_MAX_GOP, 1, 0);
	v4l2_ctrl_new_std(hdl, &t2sc_ctrl_ops, V4L2_CID_MPEG_VIDEO_B_FRAMES, 0, 0, 1, 0);
	v4l2_ctrl_new_std_menu(hdl, &t2sc_ctrl_ops, V4L2_CID_MPEG_VIDEO_BITRATE_MODE,
			       V4L2_MPEG_VIDEO_BITRATE_MODE_VBR,
			       ~BIT(V4L2_MPEG_VIDEO_BITRATE_MODE_VBR),
			       V4L2_MPEG_VIDEO_BITRATE_MODE_VBR);
	v4l2_ctrl_new_std_menu(hdl, &t2sc_ctrl_ops, V4L2_CID_MPEG_VIDEO_H264_PROFILE,
			       V4L2_MPEG_VIDEO_H264_PROFILE_HIGH, 0,
			       V4L2_MPEG_VIDEO_H264_PROFILE_HIGH);
	v4l2_ctrl_new_std_menu(hdl, &t2sc_ctrl_ops, V4L2_CID_MPEG_VIDEO_HEVC_PROFILE,
			       V4L2_MPEG_VIDEO_HEVC_PROFILE_MAIN, 0,
			       V4L2_MPEG_VIDEO_HEVC_PROFILE_MAIN);
	if (hdl->error) {
		int ret = hdl->error;

		v4l2_ctrl_handler_free(hdl);
		return ret;
	}
	ctx->fh.ctrl_handler = hdl;
	return 0;
}

static int t2sc_queue_init(void *priv, struct vb2_queue *src, struct vb2_queue *dst)
{
	struct t2sc_ctx *ctx = priv;
	int ret;

	src->type = V4L2_BUF_TYPE_VIDEO_OUTPUT_MPLANE;
	src->io_modes = VB2_MMAP | VB2_DMABUF | VB2_USERPTR;
	src->drv_priv = ctx;
	src->buf_struct_size = sizeof(struct t2sc_buffer);
	src->ops = &t2sc_vb2_ops;
	src->mem_ops = &vb2_vmalloc_memops;
	src->timestamp_flags = V4L2_BUF_FLAG_TIMESTAMP_COPY;
	src->lock = &ctx->video->lock;
	src->dev = ctx->video->v4l2_dev.dev;
	ret = vb2_queue_init(src);
	if (ret)
		return ret;

	dst->type = V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE;
	dst->io_modes = VB2_MMAP | VB2_DMABUF | VB2_USERPTR;
	dst->drv_priv = ctx;
	dst->buf_struct_size = sizeof(struct v4l2_m2m_buffer);
	dst->ops = &t2sc_vb2_ops;
	dst->mem_ops = &vb2_vmalloc_memops;
	dst->timestamp_flags = V4L2_BUF_FLAG_TIMESTAMP_COPY;
	dst->lock = &ctx->video->lock;
	dst->dev = ctx->video->v4l2_dev.dev;
	return vb2_queue_init(dst);
}

static int t2sc_open(struct file *file)
{
	struct t2sc_video *video = video_drvdata(file);
	struct t2sc_ctx *ctx;
	int ret;

	ctx = kzalloc(sizeof(*ctx), GFP_KERNEL);
	if (!ctx)
		return -ENOMEM;
	ctx->video = video;
	ctx->fps = T2SC_DEFAULT_FPS;
	ctx->bitrate = T2SC_DEFAULT_BITRATE;
	mutex_init(&ctx->out_lock);
	INIT_LIST_HEAD(&ctx->out_list);
	INIT_WORK(&ctx->run_work, t2sc_run_work);
	INIT_WORK(&ctx->drain_work, t2sc_drain_work);
	init_waitqueue_head(&ctx->src_wait);
	t2sc_fill_src_fmt(&ctx->src_fmt, T2SC_DEFAULT_WIDTH, T2SC_DEFAULT_HEIGHT);
	t2sc_fill_dst_fmt(&ctx->dst_fmt, T2SC_DEFAULT_WIDTH, T2SC_DEFAULT_HEIGHT,
			  V4L2_PIX_FMT_HEVC);
	ctx->dst_fmt.colorspace = ctx->src_fmt.colorspace;

	v4l2_fh_init(&ctx->fh, &video->vdev);
	ret = t2sc_init_ctrls(ctx);
	if (ret)
		goto err_fh;
	ctx->fh.m2m_ctx = v4l2_m2m_ctx_init(video->m2m_dev, ctx, t2sc_queue_init);
	if (IS_ERR(ctx->fh.m2m_ctx)) {
		ret = PTR_ERR(ctx->fh.m2m_ctx);
		goto err_ctrls;
	}
	/* A job needs only an input frame, output is delivered as it arrives. */
	v4l2_m2m_set_dst_buffered(ctx->fh.m2m_ctx, true);
	file->private_data = &ctx->fh;
	v4l2_fh_add(&ctx->fh, file);
	return 0;

err_ctrls:
	v4l2_ctrl_handler_free(&ctx->ctrls);
err_fh:
	v4l2_fh_exit(&ctx->fh);
	kfree(ctx);
	return ret;
}

static int t2sc_release(struct file *file)
{
	struct t2sc_video *video = video_drvdata(file);
	struct t2sc_ctx *ctx = fh_to_ctx(file);

	mutex_lock(&video->lock);
	v4l2_m2m_ctx_release(ctx->fh.m2m_ctx);
	mutex_unlock(&video->lock);
	v4l2_fh_del(&ctx->fh, file);
	v4l2_fh_exit(&ctx->fh);
	v4l2_ctrl_handler_free(&ctx->ctrls);
	kfree(ctx);
	return 0;
}

static const struct v4l2_file_operations t2sc_fops = {
	.owner		= THIS_MODULE,
	.open		= t2sc_open,
	.release	= t2sc_release,
	.poll		= v4l2_m2m_fop_poll,
	.unlocked_ioctl	= video_ioctl2,
	.mmap		= v4l2_m2m_fop_mmap,
};

/* Runs when the device is unregistered and the last file is closed. */
static void t2sc_video_release(struct video_device *vdev)
{
	struct t2sc_video *video = container_of(vdev, struct t2sc_video, vdev);

	destroy_workqueue(video->wq);
	v4l2_m2m_release(video->m2m_dev);
	v4l2_device_unregister(&video->v4l2_dev);
	t2sc_device_put(video->dev);
	kfree(video);
}

int t2sc_video_register(struct t2sc_device *dev)
{
	struct t2sc_video *video;
	int ret;

	video = kzalloc(sizeof(*video), GFP_KERNEL);
	if (!video)
		return -ENOMEM;
	video->dev = dev;
	mutex_init(&video->lock);

	video->wq = alloc_ordered_workqueue("t2sc-encode", WQ_HIGHPRI);
	if (!video->wq) {
		ret = -ENOMEM;
		goto err_free;
	}
	ret = v4l2_device_register(&dev->intf->dev, &video->v4l2_dev);
	if (ret)
		goto err_wq;
	video->m2m_dev = v4l2_m2m_init(&t2sc_m2m_ops);
	if (IS_ERR(video->m2m_dev)) {
		ret = PTR_ERR(video->m2m_dev);
		goto err_v4l2;
	}

	video->vdev.fops = &t2sc_fops;
	video->vdev.ioctl_ops = &t2sc_ioctl_ops;
	video->vdev.device_caps = V4L2_CAP_VIDEO_M2M_MPLANE | V4L2_CAP_STREAMING;
	video->vdev.v4l2_dev = &video->v4l2_dev;
	video->vdev.vfl_dir = VFL_DIR_M2M;
	video->vdev.lock = &video->lock;
	video->vdev.release = t2sc_video_release;
	strscpy(video->vdev.name, T2SC_NAME, sizeof(video->vdev.name));
	video_set_drvdata(&video->vdev, video);

	mutex_lock(&dev->frame_lock);
	dev->video = video;
	dev->on_frame = t2sc_on_frame;
	mutex_unlock(&dev->frame_lock);

	/* The video device keeps the transport alive until its release. */
	kref_get(&dev->ref);
	ret = video_register_device(&video->vdev, VFL_TYPE_VIDEO, -1);
	if (ret) {
		t2sc_device_put(dev);
		goto err_m2m;
	}
	dev_info(&dev->intf->dev, "encoder registered as /dev/video%d\n", video->vdev.num);
	return 0;

err_m2m:
	mutex_lock(&dev->frame_lock);
	dev->on_frame = NULL;
	dev->video = NULL;
	mutex_unlock(&dev->frame_lock);
	v4l2_m2m_release(video->m2m_dev);
err_v4l2:
	v4l2_device_unregister(&video->v4l2_dev);
err_wq:
	destroy_workqueue(video->wq);
err_free:
	kfree(video);
	return ret;
}

void t2sc_video_unregister(struct t2sc_device *dev)
{
	struct t2sc_video *video = dev->video;

	if (!video)
		return;
	mutex_lock(&dev->frame_lock);
	dev->on_frame = NULL;
	dev->video = NULL;
	mutex_unlock(&dev->frame_lock);
	/* Open files keep the video device, and so the transport, until closed. */
	video_unregister_device(&video->vdev);
}
