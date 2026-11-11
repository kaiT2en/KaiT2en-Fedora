#ifndef T2_SIDECAR_H
#define T2_SIDECAR_H

#include <linux/completion.h>
#include <linux/kref.h>
#include <linux/list.h>
#include <linux/mutex.h>
#include <linux/spinlock.h>
#include <linux/types.h>
#include <linux/wait.h>

/*
 * The T2 creates the "SideCar AVE" USB device while a RemoteXPC client is
 * connected to com.apple.videoprocessingd.encode.remote (t2remote holds that
 * connection). Every packet starts with an 8-byte header and travels as one
 * bulk transfer, ended by a short USB packet.
 */

#define T2SC_PKT_ACK		1
#define T2SC_PKT_OPEN		2
#define T2SC_PKT_CLOSE		3
#define T2SC_PKT_QUERY		4
#define T2SC_PKT_SET_PROPERTIES	5
#define T2SC_PKT_GET_PROPERTY	6
#define T2SC_PKT_PROPERTY_REPLY	7
#define T2SC_PKT_ENCODE		8
#define T2SC_PKT_ENCODE_REPLY	9
#define T2SC_PKT_COMPLETE	10
#define T2SC_PKT_FRAME		11

#define T2SC_NO_SESSION		0xffff
#define T2SC_HDR_LEN		8
#define T2SC_ENCODE_HDR_LEN	0x70
#define T2SC_FRAME_HDR_LEN	0x60

#define T2SC_FOURCC(a, b, c, d) \
	(((u32)(a) << 24) | ((u32)(b) << 16) | ((u32)(c) << 8) | (u32)(d))
#define T2SC_CODEC_H264		T2SC_FOURCC('a', 'v', 'c', '1')
#define T2SC_CODEC_HEVC		T2SC_FOURCC('h', 'v', 'c', '1')
#define T2SC_PIXFMT_NV12	T2SC_FOURCC('4', '2', '0', 'v')

/*
 * Frame option that forces a keyframe. "ForceKeyFrame" is ignored.
 */
#define T2SC_FORCE_KEYFRAME	"EncoderForceKeyframe"

struct t2sc_hdr {
	__le16 type;
	__le16 session;
	__le32 seq;
} __packed;

struct t2sc_device;
struct t2sc_video;

/* A received packet, header included. */
struct t2sc_pkt {
	struct list_head node;
	size_t len;
	u8 data[];
};

struct t2sc_waiter {
	struct list_head node;
	u32 seq;
	struct completion done;
	struct t2sc_pkt *reply;
};

/* Encoded frame handed to the video layer, Annex B. */
struct t2sc_frame {
	struct list_head node;
	u32 frame_id;
	s32 status;
	bool keyframe;
	size_t len;
	u8 data[];
};

struct t2sc_session {
	struct t2sc_device *dev;
	u16 id;
	u32 codec;
	u32 width;
	u32 height;

	/* Pool pixel buffer layout reported by the T2 */
	u32 plane_offset[2];
	u32 plane_stride[2];
	size_t packet_len;
	size_t props_len;

	/*
	 * EncodeFrame packets, laid out like the T2's pixel buffer. Only one is
	 * in flight at a time (see t2sc_session_submit), they alternate.
	 */
	u8 *tx[2];
	size_t tx_len;
	u32 tx_frame_id[2];
	unsigned int cur;
	bool inflight[2];
	struct t2sc_waiter wait[2];

	/* Parameter sets from the last format description, Annex B */
	u8 *params;
	size_t params_len;
	unsigned int nal_len_size;

	/* Closed on the T2 by a system suspend, only t2sc_session_close is left */
	bool dead;
};

/* usb.c */
struct t2sc_device {
	struct kref ref;
	struct usb_device *udev;
	struct usb_interface *intf;
	unsigned int pipe_in;
	unsigned int pipe_out;
	unsigned int max_packet;

	struct mutex tx_lock;
	bool gone;
	/* Sends are refused from the start of a system suspend until resume */
	bool suspended;

	struct task_struct *rx_thread;
	struct urb *rx_urb;
	struct completion rx_done;
	/* The receive thread waits here while the receive URB is poisoned */
	bool rx_suspended;
	wait_queue_head_t rx_wait;
	spinlock_t wait_lock;
	struct list_head waiters;
	u32 seq;

	/* Encoded frames are passed here from the receive thread. */
	struct mutex frame_lock;
	struct t2sc_session *session;
	void (*on_frame)(struct t2sc_video *video, struct t2sc_frame *frame);
	struct t2sc_video *video;
};

void t2sc_device_put(struct t2sc_device *dev);
u32 t2sc_next_seq(struct t2sc_device *dev);
int t2sc_send(struct t2sc_device *dev, void *buf, size_t len, bool vmalloced);
int t2sc_request(struct t2sc_device *dev, u16 type, u16 session, const void *payload,
		 size_t payload_len, struct t2sc_pkt **reply);
int t2sc_wait_reply(struct t2sc_device *dev, struct t2sc_waiter *w, unsigned int timeout_ms,
		    struct t2sc_pkt **reply);
void t2sc_add_waiter(struct t2sc_device *dev, struct t2sc_waiter *w, u32 seq);
void t2sc_remove_waiter(struct t2sc_device *dev, struct t2sc_waiter *w);

/* session.c */
struct t2sc_session_params {
	u32 codec;
	u32 width;
	u32 height;
	u32 fps;
	u32 bitrate;
	u32 gop_size;
};

int t2sc_session_open(struct t2sc_device *dev, const struct t2sc_session_params *p,
		      struct t2sc_session **result);
int t2sc_session_set_bitrate(struct t2sc_session *s, u32 bitrate);
void t2sc_session_prepare(struct t2sc_session *s, u32 frame_id, u64 pts_us, u32 fps,
			  const u8 *y, u32 y_stride, const u8 *uv, u32 uv_stride,
			  bool force_keyframe);
int t2sc_session_submit(struct t2sc_session *s);
int t2sc_session_complete(struct t2sc_session *s);
void t2sc_session_close(struct t2sc_session *s);
void t2sc_session_handle_frame(struct t2sc_device *dev, struct t2sc_pkt *pkt);

/* video.c */
int t2sc_video_register(struct t2sc_device *dev);
void t2sc_video_unregister(struct t2sc_device *dev);

#endif
