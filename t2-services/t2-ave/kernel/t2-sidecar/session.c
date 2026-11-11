#define pr_fmt(fmt) KBUILD_MODNAME ": " fmt

#include "bplist.h"
#include "sidecar.h"

#include <linux/slab.h>
#include <linux/unaligned.h>
#include <linux/vmalloc.h>

#define T2SC_PLIST_MAX		512
#define T2SC_SESSION_TIMEOUT_MS	2000

struct t2sc_cmtime {
	__le64 value;
	__le32 timescale;
	__le32 flags;
	__le64 epoch;
} __packed;

struct t2sc_open_hdr {
	__le32 version;
	__le32 width;
	__le32 height;
	__le32 codec;
	__le32 spec_len;
	__le32 attrs_len;
} __packed;

struct t2sc_encode_hdr {
	__le32 frame_id;
	struct t2sc_cmtime pts;
	struct t2sc_cmtime duration;
	__le32 pixfmt;
	__le32 props_len;
	__le32 attachments_len;
	__le32 plane_offset[4];
	__le32 plane_size[4];
	__le32 plane_stride[4];
} __packed;

static_assert(sizeof(struct t2sc_encode_hdr) == T2SC_ENCODE_HDR_LEN);

static void t2sc_cmtime_set(struct t2sc_cmtime *t, s64 value, u32 timescale)
{
	t->value = cpu_to_le64(value);
	t->timescale = cpu_to_le32(timescale);
	t->flags = cpu_to_le32(1);	/* valid */
	t->epoch = 0;
}

/* Status word right after the header of an ACK, property or encode reply. */
static int t2sc_reply_status(const struct t2sc_pkt *reply, s32 *status)
{
	if (reply->len < T2SC_HDR_LEN + 4)
		return -EPROTO;
	*status = get_unaligned_le32(reply->data + T2SC_HDR_LEN);
	return 0;
}

static int t2sc_set_properties(struct t2sc_session *s, const struct t2sc_bpw *w)
{
	struct t2sc_pkt *reply;
	s32 status;
	u8 *plist;
	int len, ret;

	if (READ_ONCE(s->dead))
		return -ENODEV;
	plist = kmalloc(T2SC_PLIST_MAX, GFP_KERNEL);
	if (!plist)
		return -ENOMEM;
	len = t2sc_bpw_dict(w, plist, T2SC_PLIST_MAX);
	ret = len < 0 ? len : t2sc_request(s->dev, T2SC_PKT_SET_PROPERTIES, s->id, plist, len, &reply);
	kfree(plist);
	if (ret)
		return ret;
	ret = t2sc_reply_status(reply, &status);
	kvfree(reply);
	if (!ret && status) {
		pr_warn("SetProperties failed with status %d\n", status);
		ret = -EIO;
	}
	return ret;
}

static int t2sc_get_layout(struct t2sc_session *s)
{
	struct t2sc_pkt *reply;
	struct t2sc_bpr r;
	u8 plist[64];
	s32 status = 0;
	int len, ret, i;

	len = t2sc_bpw_string("PixelBufferLayout", plist, sizeof(plist));
	if (len < 0)
		return len;
	ret = t2sc_request(s->dev, T2SC_PKT_GET_PROPERTY, s->id, plist, len, &reply);
	if (ret)
		return ret;
	ret = t2sc_reply_status(reply, &status);
	if (!ret && status)
		ret = -EIO;
	if (!ret)
		ret = t2sc_bpr_open(&r, reply->data + T2SC_HDR_LEN + 4,
				    reply->len - T2SC_HDR_LEN - 4);
	for (i = 0; !ret && i < 2; i++) {
		s64 plane = t2sc_bpr_array_get(&r, t2sc_bpr_root(&r), i);
		s64 offset, stride;

		ret = plane < 0 ? plane :
		      t2sc_bpr_int(&r, t2sc_bpr_dict_get(&r, plane, "ByteOffset"), &offset) ?:
		      t2sc_bpr_int(&r, t2sc_bpr_dict_get(&r, plane, "BytesPerRow"), &stride);
		if (!ret && (offset <= 0 || stride <= 0 || offset > SZ_256M || stride > SZ_64K))
			ret = -EPROTO;
		if (!ret) {
			s->plane_offset[i] = offset;
			s->plane_stride[i] = stride;
		}
	}
	kvfree(reply);
	if (ret)
		pr_err("PixelBufferLayout unusable (%d, status %d)\n", ret, status);
	return ret;
}

int t2sc_session_open(struct t2sc_device *dev, const struct t2sc_session_params *p,
		      struct t2sc_session **result)
{
	struct t2sc_open_hdr *hdr;
	struct t2sc_session *s;
	struct t2sc_pkt *reply;
	struct t2sc_bpw w;
	size_t y_size, uv_size;
	int spec_len, attrs_len, ret;
	s32 status;
	u8 *buf;

	s = kzalloc(sizeof(*s), GFP_KERNEL);
	buf = kmalloc(sizeof(*hdr) + 2 * T2SC_PLIST_MAX, GFP_KERNEL);
	if (!s || !buf) {
		ret = -ENOMEM;
		goto err;
	}
	s->dev = dev;
	s->codec = p->codec;
	s->width = p->width;
	s->height = p->height;

	/* Without a usage the T2's AVE client refuses to start ("usage is not set"). */
	t2sc_bpw_init(&w);
	t2sc_bpw_add(&w, "EncoderUsage", T2SC_BPW_INT, 1);
	spec_len = t2sc_bpw_dict(&w, buf + sizeof(*hdr), T2SC_PLIST_MAX);
	/* AVE rejects pool buffers that are not IOSurface backed. */
	t2sc_bpw_init(&w);
	t2sc_bpw_add(&w, "PixelFormatType", T2SC_BPW_INT, T2SC_PIXFMT_NV12);
	t2sc_bpw_add(&w, "Width", T2SC_BPW_INT, p->width);
	t2sc_bpw_add(&w, "Height", T2SC_BPW_INT, p->height);
	t2sc_bpw_add(&w, "IOSurfaceProperties", T2SC_BPW_EMPTY_DICT, 0);
	attrs_len = spec_len < 0 ? spec_len :
		    t2sc_bpw_dict(&w, buf + sizeof(*hdr) + spec_len, T2SC_PLIST_MAX);
	if (attrs_len < 0) {
		ret = attrs_len;
		goto err;
	}
	hdr = (struct t2sc_open_hdr *)buf;
	hdr->version = cpu_to_le32(1);
	hdr->width = cpu_to_le32(p->width);
	hdr->height = cpu_to_le32(p->height);
	hdr->codec = cpu_to_le32(p->codec);
	hdr->spec_len = cpu_to_le32(spec_len);
	hdr->attrs_len = cpu_to_le32(attrs_len);

	ret = t2sc_request(dev, T2SC_PKT_OPEN, T2SC_NO_SESSION, buf,
			   sizeof(*hdr) + spec_len + attrs_len, &reply);
	if (ret)
		goto err;
	ret = t2sc_reply_status(reply, &status);
	s->id = le16_to_cpu(((struct t2sc_hdr *)reply->data)->session);
	kvfree(reply);
	if (!ret && status) {
		pr_err("OpenSession %ux%u failed with status %d\n", p->width, p->height, status);
		ret = -EIO;
	}
	if (ret)
		goto err;

	/*
	 * These small packets also move the T2's pending USB read from the
	 * buffer it posted before OpenSession into a pool pixel buffer, which
	 * the first frame must land in.
	 */
	t2sc_bpw_init(&w);
	t2sc_bpw_add(&w, "RealTime", T2SC_BPW_BOOL, 1);
	t2sc_bpw_add(&w, "AllowFrameReordering", T2SC_BPW_BOOL, 0);
	t2sc_bpw_add(&w, "ExpectedFrameRate", T2SC_BPW_INT, p->fps);
	t2sc_bpw_add(&w, "AverageBitRate", T2SC_BPW_INT, p->bitrate);
	if (p->gop_size)
		t2sc_bpw_add(&w, "MaxKeyFrameInterval", T2SC_BPW_INT, p->gop_size);
	ret = t2sc_set_properties(s, &w);
	if (!ret)
		ret = t2sc_get_layout(s);
	if (ret)
		goto err_close;

	t2sc_bpw_init(&w);
	t2sc_bpw_add(&w, T2SC_FORCE_KEYFRAME, T2SC_BPW_BOOL, 0);
	s->props_len = t2sc_bpw_dict(&w, buf, T2SC_PLIST_MAX);
	y_size = (size_t)s->plane_stride[0] * p->height;
	uv_size = (size_t)s->plane_stride[1] * DIV_ROUND_UP(p->height, 2);
	if (T2SC_HDR_LEN + T2SC_ENCODE_HDR_LEN + s->props_len > s->plane_offset[0] ||
	    s->plane_offset[0] + y_size > s->plane_offset[1] ||
	    s->plane_stride[0] < p->width || s->plane_stride[1] < p->width) {
		pr_err("PixelBufferLayout %u/%u %u/%u does not fit %ux%u\n",
		       s->plane_offset[0], s->plane_stride[0], s->plane_offset[1],
		       s->plane_stride[1], p->width, p->height);
		ret = -EPROTO;
		goto err_close;
	}
	s->tx_len = s->plane_offset[1] + uv_size;
	/* one byte of slack for t2sc_send's pad byte */
	s->tx[0] = vzalloc(s->tx_len + 1);
	s->tx[1] = vzalloc(s->tx_len + 1);
	if (!s->tx[0] || !s->tx[1]) {
		vfree(s->tx[0]);
		vfree(s->tx[1]);
		ret = -ENOMEM;
		goto err_close;
	}

	mutex_lock(&dev->frame_lock);
	dev->session = s;
	mutex_unlock(&dev->frame_lock);
	kfree(buf);
	*result = s;
	pr_info("session %#x: %ux%u %s, %u fps, %u bit/s\n", s->id, p->width, p->height,
		p->codec == T2SC_CODEC_HEVC ? "HEVC" : "H.264", p->fps, p->bitrate);
	return 0;

err_close:
	if (!t2sc_request(dev, T2SC_PKT_CLOSE, s->id, NULL, 0, &reply))
		kvfree(reply);
err:
	kfree(buf);
	kfree(s);
	return ret;
}

int t2sc_session_set_bitrate(struct t2sc_session *s, u32 bitrate)
{
	struct t2sc_bpw w;

	t2sc_bpw_init(&w);
	t2sc_bpw_add(&w, "AverageBitRate", T2SC_BPW_INT, bitrate);
	return t2sc_set_properties(s, &w);
}

/* Copies one frame into the EncodeFrame packet, the source can be released after. */
void t2sc_session_prepare(struct t2sc_session *s, u32 frame_id, u64 pts_us, u32 fps,
			  const u8 *y, u32 y_stride, const u8 *uv, u32 uv_stride,
			  bool force_keyframe)
{
	u8 *tx = s->tx[s->cur];
	struct t2sc_encode_hdr *eh = (void *)(tx + T2SC_HDR_LEN);
	u32 uv_rows = DIV_ROUND_UP(s->height, 2);
	struct t2sc_bpw bw;
	u32 row;

	s->tx_frame_id[s->cur] = frame_id;

	memset(eh, 0, sizeof(*eh));
	eh->frame_id = cpu_to_le32(frame_id);
	t2sc_cmtime_set(&eh->pts, pts_us, USEC_PER_SEC);
	t2sc_cmtime_set(&eh->duration, 1, fps ?: 60);
	eh->pixfmt = cpu_to_le32(T2SC_PIXFMT_NV12);
	eh->props_len = cpu_to_le32(s->props_len);
	eh->plane_offset[0] = cpu_to_le32(s->plane_offset[0]);
	eh->plane_offset[1] = cpu_to_le32(s->plane_offset[1]);
	eh->plane_size[0] = cpu_to_le32(s->plane_stride[0] * s->height);
	eh->plane_size[1] = cpu_to_le32(s->plane_stride[1] * uv_rows);
	eh->plane_stride[0] = cpu_to_le32(s->plane_stride[0]);
	eh->plane_stride[1] = cpu_to_le32(s->plane_stride[1]);

	/* The properties must never be empty, the T2 looks keys up in them. */
	t2sc_bpw_init(&bw);
	t2sc_bpw_add(&bw, T2SC_FORCE_KEYFRAME, T2SC_BPW_BOOL, force_keyframe);
	t2sc_bpw_dict(&bw, tx + T2SC_HDR_LEN + sizeof(*eh), s->props_len);

	for (row = 0; row < s->height; row++)
		memcpy(tx + s->plane_offset[0] + row * s->plane_stride[0],
		       y + row * y_stride, s->width);
	for (row = 0; row < uv_rows; row++)
		memcpy(tx + s->plane_offset[1] + row * s->plane_stride[1],
		       uv + row * uv_stride, s->width);
}

/* Waits until the T2 has taken the frame in tx[i], if one is in flight. */
static int t2sc_wait_frame(struct t2sc_session *s, unsigned int i)
{
	struct t2sc_pkt *reply;
	s32 status;
	int ret;

	if (!s->inflight[i])
		return 0;
	s->inflight[i] = false;
	ret = t2sc_wait_reply(s->dev, &s->wait[i], T2SC_SESSION_TIMEOUT_MS, &reply);
	if (ret)
		return ret;
	ret = t2sc_reply_status(reply, &status);
	kvfree(reply);
	if (!ret && status) {
		pr_warn("EncodeFrame %u failed with status %d\n", s->tx_frame_id[i], status);
		ret = -EIO;
	}
	return ret;
}

static int t2sc_wait_all(struct t2sc_session *s)
{
	int ret = t2sc_wait_frame(s, 0);

	return t2sc_wait_frame(s, 1) ?: ret;
}

/*
 * Sends the prepared packet and waits until the T2 has taken it. With a second
 * frame sent before the first was answered, the T2 stopped reading the bulk
 * OUT pipe altogether, so only one frame is ever in flight.
 */
int t2sc_session_submit(struct t2sc_session *s)
{
	unsigned int i = s->cur;
	struct t2sc_hdr *hdr = (void *)s->tx[i];
	u32 seq;
	int ret;

	if (READ_ONCE(s->dead))
		return -ENODEV;
	seq = t2sc_next_seq(s->dev);
	hdr->type = cpu_to_le16(T2SC_PKT_ENCODE);
	hdr->session = cpu_to_le16(s->id);
	hdr->seq = cpu_to_le32(seq);

	t2sc_add_waiter(s->dev, &s->wait[i], seq);
	ret = t2sc_send(s->dev, s->tx[i], s->tx_len, true);
	if (ret) {
		t2sc_remove_waiter(s->dev, &s->wait[i]);
		return ret;
	}
	s->inflight[i] = true;
	s->cur = i ^ 1;
	return t2sc_wait_frame(s, i);
}

int t2sc_session_complete(struct t2sc_session *s)
{
	struct t2sc_pkt *reply;
	s32 status;
	int ret;

	if (READ_ONCE(s->dead))
		return -ENODEV;
	t2sc_wait_all(s);
	ret = t2sc_request(s->dev, T2SC_PKT_COMPLETE, s->id, NULL, 0, &reply);
	if (ret)
		return ret;
	ret = t2sc_reply_status(reply, &status);
	kvfree(reply);
	return ret ?: (status ? -EIO : 0);
}

void t2sc_session_close(struct t2sc_session *s)
{
	struct t2sc_device *dev = s->dev;
	struct t2sc_pkt *reply;
	unsigned int i;

	/* The T2 crashed when its peer vanished with frames unacknowledged;
	 * frames keep being acknowledged by the receive thread after this.
	 */
	if (!READ_ONCE(s->dead)) {
		t2sc_wait_all(s);
		if (!t2sc_request(dev, T2SC_PKT_CLOSE, s->id, NULL, 0, &reply))
			kvfree(reply);
	} else {
		/* Replies the suspend did not wait for will never come */
		for (i = 0; i < ARRAY_SIZE(s->inflight); i++) {
			if (s->inflight[i]) {
				s->inflight[i] = false;
				t2sc_remove_waiter(dev, &s->wait[i]);
			}
		}
	}
	mutex_lock(&dev->frame_lock);
	if (dev->session == s)
		dev->session = NULL;
	mutex_unlock(&dev->frame_lock);
	pr_info("session %#x closed\n", s->id);
	vfree(s->tx[0]);
	vfree(s->tx[1]);
	kfree(s->params);
	kfree(s);
}

/* Parameter sets of an hvcC or avcC record as Annex B. */
static int t2sc_parse_config(struct t2sc_session *s, const u8 *cfg, size_t len, bool hevc)
{
	size_t pos, out_len = 0, need;
	unsigned int arrays, a, n, i;
	u8 *out;

	/* Two passes: size, then copy. */
	for (out = NULL;; ) {
		out_len = 0;
		if (hevc) {
			if (len < 23)
				return -EPROTO;
			arrays = cfg[22];
			pos = 23;
		} else {
			if (len < 6)
				return -EPROTO;
			arrays = 2;
			pos = 5;
		}
		for (a = 0; a < arrays; a++) {
			if (hevc) {
				if (pos + 3 > len)
					return -EPROTO;
				n = get_unaligned_be16(cfg + pos + 1);
				pos += 3;
			} else {
				if (pos + 1 > len)
					return -EPROTO;
				n = a == 0 ? cfg[pos] & 0x1f : cfg[pos];
				pos += 1;
			}
			for (i = 0; i < n; i++) {
				if (pos + 2 > len)
					return -EPROTO;
				need = get_unaligned_be16(cfg + pos);
				if (pos + 2 + need > len)
					return -EPROTO;
				if (out) {
					memcpy(out + out_len, "\0\0\0\1", 4);
					memcpy(out + out_len + 4, cfg + pos + 2, need);
				}
				out_len += 4 + need;
				pos += 2 + need;
			}
		}
		if (out)
			break;
		out = kmalloc(out_len, GFP_KERNEL);
		if (!out)
			return -ENOMEM;
	}
	kfree(s->params);
	s->params = out;
	s->params_len = out_len;
	s->nal_len_size = ((hevc ? cfg[21] : cfg[4]) & 3) + 1;
	return 0;
}

static void t2sc_parse_format(struct t2sc_session *s, const u8 *plist, size_t len)
{
	struct t2sc_bpr r;
	const u8 *cfg;
	size_t cfg_len;
	s64 atoms, obj;
	bool hevc;
	int ret;

	ret = t2sc_bpr_open(&r, plist, len);
	if (ret)
		goto out;
	atoms = t2sc_bpr_dict_get(&r, t2sc_bpr_dict_get(&r, t2sc_bpr_root(&r), "Extensions"),
				  "SampleDescriptionExtensionAtoms");
	obj = t2sc_bpr_dict_get(&r, atoms, "hvcC");
	hevc = obj >= 0;
	if (!hevc)
		obj = t2sc_bpr_dict_get(&r, atoms, "avcC");
	ret = t2sc_bpr_data(&r, obj, &cfg, &cfg_len) ?: t2sc_parse_config(s, cfg, cfg_len, hevc);
out:
	if (ret)
		pr_warn("format description without usable parameter sets (%d)\n", ret);
}

static bool t2sc_is_sync(const u8 *plist, size_t len)
{
	struct t2sc_bpr r;
	bool not_sync;

	if (!len || t2sc_bpr_open(&r, plist, len))
		return true;
	if (t2sc_bpr_bool(&r, t2sc_bpr_dict_get(&r, t2sc_bpr_root(&r), "NotSync"), &not_sync))
		return true;
	return !not_sync;
}

/* Length-prefixed NAL units to Annex B, parameter sets first on sync frames. */
static struct t2sc_frame *t2sc_build_frame(struct t2sc_session *s, const u8 *data, size_t len,
					   bool sync)
{
	unsigned int nls = s->nal_len_size;
	size_t pos, out_len = sync ? s->params_len : 0;
	struct t2sc_frame *f;
	u8 *out;

	if (!nls)
		return ERR_PTR(-EPROTO);
	for (pos = 0; pos < len; ) {
		size_t n;

		if (pos + nls > len)
			return ERR_PTR(-EPROTO);
		n = nls == 4 ? get_unaligned_be32(data + pos) :
		    nls == 2 ? get_unaligned_be16(data + pos) : data[pos];
		if (n > len - pos - nls)
			return ERR_PTR(-EPROTO);
		out_len += 4 + n;
		pos += nls + n;
	}

	f = kvmalloc(struct_size(f, data, out_len), GFP_KERNEL);
	if (!f)
		return ERR_PTR(-ENOMEM);
	f->len = out_len;
	out = f->data;
	if (sync && s->params_len) {
		memcpy(out, s->params, s->params_len);
		out += s->params_len;
	}
	for (pos = 0; pos < len; ) {
		size_t n = nls == 4 ? get_unaligned_be32(data + pos) :
			   nls == 2 ? get_unaligned_be16(data + pos) : data[pos];

		memcpy(out, "\0\0\0\1", 4);
		memcpy(out + 4, data + pos + nls, n);
		out += 4 + n;
		pos += nls + n;
	}
	return f;
}

void t2sc_session_handle_frame(struct t2sc_device *dev, struct t2sc_pkt *pkt)
{
	const struct t2sc_hdr *hdr = (const void *)pkt->data;
	const u8 *body = pkt->data + T2SC_HDR_LEN;
	size_t body_len = pkt->len - T2SC_HDR_LEN;
	u32 fmt_len, att_len, data_len, frame_id;
	struct t2sc_session *s;
	struct t2sc_frame *f;
	u8 *ack;
	s32 status;
	bool sync;

	/*
	 * The T2 blocks its emit queue for up to a second per unacknowledged
	 * frame, so acknowledge first, whatever follows.
	 */
	ack = kmalloc(T2SC_HDR_LEN + 4 + 1, GFP_KERNEL);
	if (ack) {
		memcpy(ack, hdr, T2SC_HDR_LEN);
		((struct t2sc_hdr *)ack)->type = cpu_to_le16(T2SC_PKT_ACK);
		put_unaligned_le32(0, ack + T2SC_HDR_LEN);
		t2sc_send(dev, ack, T2SC_HDR_LEN + 4, false);
		kfree(ack);
	}

	if (body_len < T2SC_FRAME_HDR_LEN)
		return;
	frame_id = get_unaligned_le32(body);
	status = get_unaligned_le32(body + 4);
	fmt_len = get_unaligned_le32(body + 0x54);
	att_len = get_unaligned_le32(body + 0x58);
	data_len = get_unaligned_le32(body + 0x5c);
	if ((u64)T2SC_FRAME_HDR_LEN + fmt_len + att_len + data_len > body_len)
		return;
	body += T2SC_FRAME_HDR_LEN;

	mutex_lock(&dev->frame_lock);
	s = dev->session;
	if (!s || le16_to_cpu(hdr->session) != s->id)
		goto out;
	if (fmt_len)
		t2sc_parse_format(s, body, fmt_len);
	sync = t2sc_is_sync(body + fmt_len, att_len);
	if (status || !data_len) {
		f = kvzalloc(sizeof(*f), GFP_KERNEL);
		if (!f)
			goto out;
	} else {
		f = t2sc_build_frame(s, body + fmt_len + att_len, data_len, sync);
		if (IS_ERR(f)) {
			pr_warn("frame %u: bad bitstream (%ld)\n", frame_id, PTR_ERR(f));
			goto out;
		}
	}
	f->frame_id = frame_id;
	f->status = status;
	f->keyframe = sync;
	if (dev->on_frame)
		dev->on_frame(dev->video, f);
	else
		kvfree(f);
out:
	mutex_unlock(&dev->frame_lock);
}
