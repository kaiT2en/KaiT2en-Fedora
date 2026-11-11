#define pr_fmt(fmt) KBUILD_MODNAME ": " fmt

#include "sidecar.h"

#include <linux/delay.h>
#include <linux/kthread.h>
#include <linux/mm.h>
#include <linux/module.h>
#include <linux/scatterlist.h>
#include <linux/slab.h>
#include <linux/usb.h>
#include <linux/vmalloc.h>

#define T2SC_VENDOR		0x05ac
#define T2SC_PRODUCT		0x9312
#define T2SC_RX_CHUNK		(64 * 1024)
#define T2SC_RX_MAX		(32 * 1024 * 1024)
#define T2SC_TX_TIMEOUT_MS	2000
#define T2SC_REPLY_TIMEOUT_MS	2000

static void t2sc_device_release(struct kref *ref)
{
	struct t2sc_device *dev = container_of(ref, struct t2sc_device, ref);

	usb_free_urb(dev->rx_urb);
	usb_put_dev(dev->udev);
	kfree(dev);
}

void t2sc_device_put(struct t2sc_device *dev)
{
	kref_put(&dev->ref, t2sc_device_release);
}

u32 t2sc_next_seq(struct t2sc_device *dev)
{
	u32 seq;

	spin_lock(&dev->wait_lock);
	seq = ++dev->seq;
	spin_unlock(&dev->wait_lock);
	return seq;
}

static void t2sc_urb_complete(struct urb *urb)
{
	complete(urb->context);
}

/*
 * Each packet must end with a short USB packet. A zero-length packet after a
 * multiple of wMaxPacketSize timed out through apple-t2-vhci-hcd, so such packets are
 * sent with one trailing pad byte; the T2 takes all lengths from the contents.
 * Callers provide one byte of slack after len.
 */
static size_t t2sc_wire_len(struct t2sc_device *dev, size_t len)
{
	return len % dev->max_packet ? len : len + 1;
}

static int t2sc_send_vmalloc(struct t2sc_device *dev, void *buf, size_t len)
{
	unsigned int offset = offset_in_page(buf);
	unsigned int npages = DIV_ROUND_UP(offset + len, PAGE_SIZE);
	DECLARE_COMPLETION_ONSTACK(done);
	struct sg_table sgt;
	struct page **pages;
	struct urb *urb;
	unsigned int i;
	int ret;

	pages = kvmalloc_array(npages, sizeof(*pages), GFP_KERNEL);
	if (!pages)
		return -ENOMEM;
	for (i = 0; i < npages; i++)
		pages[i] = vmalloc_to_page(buf - offset + i * PAGE_SIZE);
	ret = sg_alloc_table_from_pages(&sgt, pages, npages, offset, len, GFP_KERNEL);
	kvfree(pages);
	if (ret)
		return ret;

	urb = usb_alloc_urb(0, GFP_KERNEL);
	if (!urb) {
		ret = -ENOMEM;
		goto out_sgt;
	}
	usb_fill_bulk_urb(urb, dev->udev, dev->pipe_out, NULL, len, t2sc_urb_complete, &done);
	urb->sg = sgt.sgl;
	urb->num_sgs = sgt.orig_nents;
	ret = usb_submit_urb(urb, GFP_KERNEL);
	if (ret)
		goto out_urb;
	if (!wait_for_completion_timeout(&done, msecs_to_jiffies(T2SC_TX_TIMEOUT_MS))) {
		usb_kill_urb(urb);
		ret = -ETIMEDOUT;
	} else {
		ret = urb->status;
		if (!ret && urb->actual_length != len)
			ret = -EIO;
	}
out_urb:
	usb_free_urb(urb);
out_sgt:
	sg_free_table(&sgt);
	return ret;
}

static int __t2sc_send(struct t2sc_device *dev, void *buf, size_t len, bool vmalloced,
		       bool suspending)
{
	int actual, ret;

	len = t2sc_wire_len(dev, len);
	mutex_lock(&dev->tx_lock);
	if (dev->gone) {
		ret = -ENODEV;
	} else if (dev->suspended && !suspending) {
		ret = -EAGAIN;
	} else if (vmalloced) {
		ret = t2sc_send_vmalloc(dev, buf, len);
	} else {
		ret = usb_bulk_msg(dev->udev, dev->pipe_out, buf, len, &actual, T2SC_TX_TIMEOUT_MS);
		if (!ret && actual != len)
			ret = -EIO;
	}
	mutex_unlock(&dev->tx_lock);
	if (ret && ret != -ENODEV && ret != -EAGAIN)
		pr_warn("send of %zu bytes failed (%d)\n", len, ret);
	return ret;
}

int t2sc_send(struct t2sc_device *dev, void *buf, size_t len, bool vmalloced)
{
	return __t2sc_send(dev, buf, len, vmalloced, false);
}

void t2sc_add_waiter(struct t2sc_device *dev, struct t2sc_waiter *w, u32 seq)
{
	w->seq = seq;
	w->reply = NULL;
	init_completion(&w->done);
	spin_lock(&dev->wait_lock);
	list_add_tail(&w->node, &dev->waiters);
	spin_unlock(&dev->wait_lock);
}

void t2sc_remove_waiter(struct t2sc_device *dev, struct t2sc_waiter *w)
{
	spin_lock(&dev->wait_lock);
	if (!list_empty(&w->node))
		list_del_init(&w->node);
	spin_unlock(&dev->wait_lock);
	kvfree(w->reply);
	w->reply = NULL;
}

int t2sc_wait_reply(struct t2sc_device *dev, struct t2sc_waiter *w, unsigned int timeout_ms,
		    struct t2sc_pkt **reply)
{
	if (!wait_for_completion_timeout(&w->done, msecs_to_jiffies(timeout_ms))) {
		t2sc_remove_waiter(dev, w);
		return -ETIMEDOUT;
	}
	if (!w->reply)
		return -ENODEV;
	*reply = w->reply;
	w->reply = NULL;
	return 0;
}

static int __t2sc_request(struct t2sc_device *dev, u16 type, u16 session, const void *payload,
			  size_t payload_len, struct t2sc_pkt **reply, bool suspending)
{
	struct t2sc_waiter w;
	struct t2sc_hdr *hdr;
	u8 *buf;
	int ret;

	buf = kmalloc(T2SC_HDR_LEN + payload_len + 1, GFP_KERNEL);
	if (!buf)
		return -ENOMEM;
	hdr = (struct t2sc_hdr *)buf;
	hdr->type = cpu_to_le16(type);
	hdr->session = cpu_to_le16(session);
	hdr->seq = cpu_to_le32(t2sc_next_seq(dev));
	if (payload_len)
		memcpy(buf + T2SC_HDR_LEN, payload, payload_len);

	t2sc_add_waiter(dev, &w, le32_to_cpu(hdr->seq));
	ret = __t2sc_send(dev, buf, T2SC_HDR_LEN + payload_len, false, suspending);
	kfree(buf);
	if (ret) {
		t2sc_remove_waiter(dev, &w);
		return ret;
	}
	return t2sc_wait_reply(dev, &w, T2SC_REPLY_TIMEOUT_MS, reply);
}

int t2sc_request(struct t2sc_device *dev, u16 type, u16 session, const void *payload,
		 size_t payload_len, struct t2sc_pkt **reply)
{
	return __t2sc_request(dev, type, session, payload, payload_len, reply, false);
}

static void t2sc_dispatch(struct t2sc_device *dev, struct t2sc_pkt *pkt)
{
	const struct t2sc_hdr *hdr = (const void *)pkt->data;
	u16 type = le16_to_cpu(hdr->type);
	u32 seq = le32_to_cpu(hdr->seq);
	struct t2sc_waiter *w;

	switch (type) {
	case T2SC_PKT_ACK:
	case T2SC_PKT_PROPERTY_REPLY:
	case T2SC_PKT_ENCODE_REPLY:
		spin_lock(&dev->wait_lock);
		list_for_each_entry(w, &dev->waiters, node) {
			if (w->seq == seq) {
				list_del_init(&w->node);
				w->reply = pkt;
				complete(&w->done);
				spin_unlock(&dev->wait_lock);
				return;
			}
		}
		spin_unlock(&dev->wait_lock);
		dev_dbg(&dev->intf->dev, "unexpected reply type %u seq %u\n", type, seq);
		break;
	case T2SC_PKT_FRAME:
		t2sc_session_handle_frame(dev, pkt);
		break;
	default:
		dev_dbg(&dev->intf->dev, "ignoring packet type %u\n", type);
		break;
	}
	kvfree(pkt);
}

/*
 * Packets end with a short transfer. The T2 writes them in chunks of up to
 * 512 KiB, each read here may return part of one.
 */
static int t2sc_rx_thread(void *data)
{
	struct t2sc_device *dev = data;
	size_t cap = 256 * 1024, len = 0;
	u8 *chunk, *asm_buf;
	int ret;

	chunk = kmalloc(T2SC_RX_CHUNK, GFP_KERNEL);
	asm_buf = kvmalloc(cap, GFP_KERNEL);
	if (!chunk || !asm_buf)
		goto out;

	while (!kthread_should_stop() && !READ_ONCE(dev->gone)) {
		struct t2sc_pkt *pkt;
		unsigned int got;

		if (READ_ONCE(dev->rx_suspended)) {
			len = 0;
			wait_event_interruptible(dev->rx_wait, !READ_ONCE(dev->rx_suspended) ||
						 READ_ONCE(dev->gone) || kthread_should_stop());
			continue;
		}

		reinit_completion(&dev->rx_done);
		usb_fill_bulk_urb(dev->rx_urb, dev->udev, dev->pipe_in, chunk, T2SC_RX_CHUNK,
				  t2sc_urb_complete, &dev->rx_done);
		ret = usb_submit_urb(dev->rx_urb, GFP_KERNEL);
		if (ret) {
			/* The suspend poisons the URB, wait for the resume */
			if (READ_ONCE(dev->rx_suspended))
				continue;
			break;
		}
		wait_for_completion(&dev->rx_done);
		ret = dev->rx_urb->status;
		got = dev->rx_urb->actual_length;
		if (ret == -ENOENT || ret == -ECONNRESET || ret == -ESHUTDOWN || ret == -ENODEV) {
			if (READ_ONCE(dev->rx_suspended))
				continue;
			break;
		}
		if (ret) {
			dev_warn(&dev->intf->dev, "receive failed (%d), dropping partial packet\n", ret);
			len = 0;
			continue;
		}

		if (len + got > cap) {
			size_t want = max(cap * 2, len + got);
			u8 *bigger;

			if (want > T2SC_RX_MAX) {
				dev_warn(&dev->intf->dev, "packet over %u bytes, dropping\n", T2SC_RX_MAX);
				len = 0;
				continue;
			}
			bigger = kvmalloc(want, GFP_KERNEL);
			if (!bigger) {
				len = 0;
				continue;
			}
			memcpy(bigger, asm_buf, len);
			kvfree(asm_buf);
			asm_buf = bigger;
			cap = want;
		}
		memcpy(asm_buf + len, chunk, got);
		len += got;
		if (got == T2SC_RX_CHUNK)
			continue;

		if (len >= T2SC_HDR_LEN) {
			pkt = kvmalloc(struct_size(pkt, data, len), GFP_KERNEL);
			if (pkt) {
				pkt->len = len;
				memcpy(pkt->data, asm_buf, len);
				t2sc_dispatch(dev, pkt);
			}
		}
		len = 0;
	}
out:
	kfree(chunk);
	kvfree(asm_buf);
	while (!kthread_should_stop())
		schedule_timeout_interruptible(HZ);
	return 0;
}

static void t2sc_fail_waiters(struct t2sc_device *dev)
{
	struct t2sc_waiter *w, *tmp;

	spin_lock(&dev->wait_lock);
	list_for_each_entry_safe(w, tmp, &dev->waiters, node) {
		list_del_init(&w->node);
		complete(&w->done);
	}
	spin_unlock(&dev->wait_lock);
}

static int t2sc_probe(struct usb_interface *intf, const struct usb_device_id *id)
{
	struct usb_endpoint_descriptor *in, *out;
	struct t2sc_device *dev;
	int ret;

	ret = usb_find_common_endpoints(intf->cur_altsetting, &in, &out, NULL, NULL);
	if (ret)
		return ret;

	dev = kzalloc(sizeof(*dev), GFP_KERNEL);
	if (!dev)
		return -ENOMEM;
	kref_init(&dev->ref);
	dev->udev = usb_get_dev(interface_to_usbdev(intf));
	dev->intf = intf;
	dev->pipe_in = usb_rcvbulkpipe(dev->udev, usb_endpoint_num(in));
	dev->pipe_out = usb_sndbulkpipe(dev->udev, usb_endpoint_num(out));
	dev->max_packet = usb_endpoint_maxp(out);
	mutex_init(&dev->tx_lock);
	mutex_init(&dev->frame_lock);
	spin_lock_init(&dev->wait_lock);
	INIT_LIST_HEAD(&dev->waiters);
	init_completion(&dev->rx_done);
	init_waitqueue_head(&dev->rx_wait);
	dev->rx_urb = usb_alloc_urb(0, GFP_KERNEL);
	if (!dev->rx_urb) {
		ret = -ENOMEM;
		goto err_put;
	}
	usb_set_intfdata(intf, dev);

	dev->rx_thread = kthread_run(t2sc_rx_thread, dev, "t2sc-rx");
	if (IS_ERR(dev->rx_thread)) {
		ret = PTR_ERR(dev->rx_thread);
		goto err_put;
	}

	ret = t2sc_video_register(dev);
	if (ret)
		goto err_thread;
	dev_info(&intf->dev, "SideCar AVE encoder attached\n");
	return 0;

err_thread:
	WRITE_ONCE(dev->gone, true);
	usb_poison_urb(dev->rx_urb);
	kthread_stop(dev->rx_thread);
err_put:
	usb_set_intfdata(intf, NULL);
	t2sc_device_put(dev);
	return ret;
}

static void t2sc_disconnect(struct usb_interface *intf)
{
	struct t2sc_device *dev = usb_get_intfdata(intf);

	mutex_lock(&dev->tx_lock);
	WRITE_ONCE(dev->gone, true);
	mutex_unlock(&dev->tx_lock);
	/* Poisoning also rejects a resubmit racing with this. */
	usb_poison_urb(dev->rx_urb);
	kthread_stop(dev->rx_thread);
	t2sc_fail_waiters(dev);
	t2sc_video_unregister(dev);
	usb_set_intfdata(intf, NULL);
	t2sc_device_put(dev);
	dev_info(&intf->dev, "SideCar AVE encoder detached\n");
}

/* Waits for the replies to requests in flight, the receive thread still runs. */
static void t2sc_drain_waiters(struct t2sc_device *dev)
{
	unsigned long end = jiffies + msecs_to_jiffies(T2SC_REPLY_TIMEOUT_MS);
	bool empty;

	for (;;) {
		spin_lock(&dev->wait_lock);
		empty = list_empty(&dev->waiters);
		spin_unlock(&dev->wait_lock);
		if (empty || time_after(jiffies, end))
			return;
		msleep(10);
	}
}

/*
 * Without suspend callbacks the USB core unbinds the driver during the system
 * suspend and the T2 keeps the session and the queued transfers. Close the
 * session on the T2 and park the receive thread instead.
 */
static int t2sc_suspend(struct usb_interface *intf, pm_message_t message)
{
	struct t2sc_device *dev = usb_get_intfdata(intf);
	struct t2sc_pkt *reply;
	bool close = false;
	u16 id = 0;

	mutex_lock(&dev->tx_lock);
	dev->suspended = true;
	mutex_unlock(&dev->tx_lock);
	t2sc_drain_waiters(dev);

	mutex_lock(&dev->frame_lock);
	if (dev->session) {
		WRITE_ONCE(dev->session->dead, true);
		id = dev->session->id;
		close = true;
	}
	mutex_unlock(&dev->frame_lock);
	if (close && !__t2sc_request(dev, T2SC_PKT_CLOSE, id, NULL, 0, &reply, true)) {
		kvfree(reply);
		dev_info(&intf->dev, "session %#x closed for suspend\n", id);
	}

	WRITE_ONCE(dev->rx_suspended, true);
	usb_poison_urb(dev->rx_urb);
	t2sc_fail_waiters(dev);
	return 0;
}

static int t2sc_resume(struct usb_interface *intf)
{
	struct t2sc_device *dev = usb_get_intfdata(intf);

	usb_unpoison_urb(dev->rx_urb);
	WRITE_ONCE(dev->rx_suspended, false);
	wake_up(&dev->rx_wait);

	mutex_lock(&dev->tx_lock);
	dev->suspended = false;
	mutex_unlock(&dev->tx_lock);
	return 0;
}

static const struct usb_device_id t2sc_ids[] = {
	{ USB_DEVICE_AND_INTERFACE_INFO(T2SC_VENDOR, T2SC_PRODUCT, 0xff, 0x90, 0xff) },
	{ }
};
MODULE_DEVICE_TABLE(usb, t2sc_ids);

static struct usb_driver t2sc_driver = {
	.name = "t2-sidecar",
	.probe = t2sc_probe,
	.disconnect = t2sc_disconnect,
	.suspend = t2sc_suspend,
	.resume = t2sc_resume,
	/* The session died with the suspend, nothing else to restore */
	.reset_resume = t2sc_resume,
	.id_table = t2sc_ids,
	.soft_unbind = 1,
};
module_usb_driver(t2sc_driver);

MODULE_LICENSE("GPL");
MODULE_AUTHOR("André Eikmeyer <andre.eikmeyer@kait2en.org>");
MODULE_DESCRIPTION("Apple T2 Sidecar video encoder");
MODULE_VERSION("0.03");
