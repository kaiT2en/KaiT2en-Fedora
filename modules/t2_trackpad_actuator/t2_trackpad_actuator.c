// SPDX-License-Identifier: GPL-2.0-only
/*
 * Apple T2 trackpad Taptic Engine actuator driver.
 *
 * The T2 internal trackpad exposes its haptic actuator as a separate HID
 * interface from t2_precision_trackpad, with vendor usage page 0xff00 and
 * usage 0x0d. This driver exposes report 0x53.
 */

#define pr_fmt(fmt) KBUILD_MODNAME ": " fmt

#include <linux/hid.h>
#include <linux/module.h>
#include <linux/mutex.h>
#include <linux/slab.h>
#include <linux/string.h>
#include <linux/workqueue.h>

#include "hid-ids.h"
#include "t2_trackpad_actuator.h"

#define T2_ACTUATOR_USAGE_PAGE		0xff00
#define T2_ACTUATOR_USAGE		0x0d
#define T2_ACTUATOR_COLLECTION_USAGE	((T2_ACTUATOR_USAGE_PAGE << 16) | T2_ACTUATOR_USAGE)

#define T2_ACTUATOR_WAVEFORM_REPORT_ID	0x53
#define T2_ACTUATOR_MAX_TONES		2

/*
 * Waveforms use report 0x53.
 * byte 0 is the base waveform. bytes 1 and 2 are base parameters.
 * Each tone occupies five bytes from byte 3. Captured reports contain
 * either no tones or two tones for total payload lengths of 3 or 13 bytes.
 */
enum t2_actuator_waveform_type {
	T2_ACT_WAVE_NONE = 0,
	T2_ACT_WAVE_GAUSSIAN = 1,
};

enum t2_actuator_tone_type {
	T2_ACT_TONE_NONE = 0,
	T2_ACT_TONE_SINE = 1,
	T2_ACT_TONE_SQUARE = 2,
	T2_ACT_TONE_SAWTOOTH = 3,
};

/*
 * Actuations on macOS 15 (MacBookAir9,1, traced with DTrace) send plain
 * Gaussian reports such as 01 12 a8 with amplitudes from
 * 0x09 to 0x26. The actuator advertises ActuatorLimits AmplitudeMax=70.
 * Probe amplitudes are capped there, so diagnostics stay in the range macOS
 * uses.
 */
#define T2_ACTUATOR_AMP_MAX		70

/* Captured click reports use base amplitudes near 0x1c and 0x2d. */
static const u8 t2_actuator_click_light[] = {
	0x01, 0x1c, 0x8f, 0x01, 0x03, 0x00, 0x53, 0x37, 0x02, 0x02, 0x00, 0x18, 0x4d,
};
static const u8 t2_actuator_click_firm[] = {
	0x01, 0x2d, 0x8f, 0x01, 0x05, 0x00, 0x53, 0x37, 0x02, 0x02, 0x00, 0x30, 0x4d,
};

static unsigned int probe_type = T2_ACT_WAVE_GAUSSIAN;
static unsigned int probe_amp = 255;
static unsigned int probe_dur = 255;
static unsigned int probe_t0_type = T2_ACT_TONE_SQUARE;
static unsigned int probe_t0_amp = 255;
static unsigned int probe_t0_delay = 255;
static unsigned int probe_t0_dur = 255;
static unsigned int probe_t0_param = 255;
static unsigned int probe_t1_type = T2_ACT_TONE_SINE;
static unsigned int probe_t1_amp = 255;
static unsigned int probe_t1_delay = 255;
static unsigned int probe_t1_dur = 255;
static unsigned int probe_t1_param = 255;
module_param(probe_type, uint, 0644);
module_param(probe_amp, uint, 0644);
module_param(probe_dur, uint, 0644);
module_param(probe_t0_type, uint, 0644);
module_param(probe_t0_amp, uint, 0644);
module_param(probe_t0_delay, uint, 0644);
module_param(probe_t0_dur, uint, 0644);
module_param(probe_t0_param, uint, 0644);
module_param(probe_t1_type, uint, 0644);
module_param(probe_t1_amp, uint, 0644);
module_param(probe_t1_delay, uint, 0644);
module_param(probe_t1_dur, uint, 0644);
module_param(probe_t1_param, uint, 0644);
MODULE_PARM_DESC(probe_dur, "waveform_id 9: raw byte[2] (BaseWaveform duration), 0-255");

static int t2_actuator_encode_probe(u8 *buf, size_t buf_size)
{
	size_t len = 3 + 5 * 2;

	if (buf_size < len)
		return -ENOSPC;

	buf[0] = (u8)probe_type;
	buf[1] = (u8)clamp_t(unsigned int, probe_amp, 0, T2_ACTUATOR_AMP_MAX);
	buf[2] = (u8)clamp_t(unsigned int, probe_dur, 0, 255);

	buf[3] = (u8)probe_t0_type;
	buf[4] = (u8)clamp_t(unsigned int, probe_t0_amp, 0, T2_ACTUATOR_AMP_MAX);
	buf[5] = (u8)clamp_t(unsigned int, probe_t0_delay, 0, 255);
	buf[6] = (u8)clamp_t(unsigned int, probe_t0_dur, 0, 255);
	buf[7] = (u8)clamp_t(unsigned int, probe_t0_param, 0, 255);

	buf[8] = (u8)probe_t1_type;
	buf[9] = (u8)clamp_t(unsigned int, probe_t1_amp, 0, T2_ACTUATOR_AMP_MAX);
	buf[10] = (u8)clamp_t(unsigned int, probe_t1_delay, 0, 255);
	buf[11] = (u8)clamp_t(unsigned int, probe_t1_dur, 0, 255);
	buf[12] = (u8)clamp_t(unsigned int, probe_t1_param, 0, 255);

	return (int)len;
}

struct t2_actuator {
	struct hid_device *hdev;
};

/*
 * The bound actuator, if any. t2_precision_trackpad (or anything else)
 * reaches it through t2_actuator_fire() rather than a direct reference, so
 * neither side needs the other loaded to build or to run.
 */
static DEFINE_MUTEX(t2_actuator_lock);
static struct t2_actuator *t2_actuator_active;

struct t2_actuator_fire_work {
	struct work_struct work;
	u8 waveform_id;
	u8 strength;
};

static void t2_actuator_fire_work_fn(struct work_struct *work)
{
	struct t2_actuator_fire_work *fw =
		container_of(work, struct t2_actuator_fire_work, work);
	struct t2_actuator *act;
	u8 *buf;
	int len, ret;

	/*
	 * hid_hw_raw_request() ends up DMA-mapping this buffer for the T2
	 * VHCI transport's control transfer. A stack buffer here crashed
	 * (oops in usb_hcd_map_urb_for_dma via usb_start_wait_urb).
	 */
	buf = kmalloc(1 + 3 + 5 * T2_ACTUATOR_MAX_TONES, GFP_KERNEL);
	if (!buf) {
		kfree(fw);
		return;
	}

	mutex_lock(&t2_actuator_lock);
	act = t2_actuator_active;
	if (act) {
		if (fw->waveform_id == 9) {
			len = t2_actuator_encode_probe(&buf[1],
							1 + 3 + 5 * T2_ACTUATOR_MAX_TONES - 1);
		} else {
			const u8 *body = fw->strength == 0 ? t2_actuator_click_light :
							      t2_actuator_click_firm;
			size_t body_len = fw->strength == 0 ? sizeof(t2_actuator_click_light) :
							       sizeof(t2_actuator_click_firm);

			memcpy(&buf[1], body, body_len);
			len = (int)body_len;
		}
		if (len >= 0) {
			buf[0] = T2_ACTUATOR_WAVEFORM_REPORT_ID;
			ret = hid_hw_raw_request(act->hdev, T2_ACTUATOR_WAVEFORM_REPORT_ID,
						  buf, len + 1, HID_OUTPUT_REPORT,
						  HID_REQ_SET_REPORT);
			/*
			 * The actuator stalls a report while the previous
			 * waveform still plays. macOS resends it once right
			 * away, which then succeeds.
			 */
			if (ret == -EPIPE)
				ret = hid_hw_raw_request(act->hdev,
							 T2_ACTUATOR_WAVEFORM_REPORT_ID,
							 buf, len + 1, HID_OUTPUT_REPORT,
							 HID_REQ_SET_REPORT);
			if (ret < 0)
				hid_warn_ratelimited(act->hdev,
						     "actuator report failed: %d\n", ret);
		}
	}
	mutex_unlock(&t2_actuator_lock);

	kfree(buf);
	kfree(fw);
}

/*
 * Send a waveform to the actuator. waveform_id 1 plays
 * the DTrace-captured click body.
 */
int t2_actuator_fire(u8 waveform_id, u8 strength)
{
	struct t2_actuator_fire_work *fw;

	if (waveform_id != 1 && waveform_id != 9)
		return -ENOENT;

	fw = kmalloc(sizeof(*fw), GFP_ATOMIC);
	if (!fw)
		return -ENOMEM;

	fw->waveform_id = waveform_id;
	fw->strength = strength;
	INIT_WORK(&fw->work, t2_actuator_fire_work_fn);
	schedule_work(&fw->work);

	return 0;
}
EXPORT_SYMBOL_GPL(t2_actuator_fire);

/* Userspace playback accepts only DTrace-captured click reports. */
static int t2_actuator_play_set(const char *val, const struct kernel_param *kp)
{
	if (sysfs_streq(val, "light"))
		return t2_actuator_fire(1, 0);
	if (sysfs_streq(val, "firm"))
		return t2_actuator_fire(1, 1);

	return -EINVAL;
}

static const struct kernel_param_ops t2_actuator_play_ops = {
	.set = t2_actuator_play_set,
};

module_param_cb(play, &t2_actuator_play_ops, NULL, 0200);
MODULE_PARM_DESC(play, "write light or firm to play a captured click waveform");

/* Manual trigger for testing: `echo 1|9 > .../parameters/test_fire` */
static int t2_actuator_test_fire_set(const char *val, const struct kernel_param *kp)
{
	unsigned int waveform_id;
	int ret;

	ret = kstrtouint(val, 0, &waveform_id);
	if (ret)
		return ret;

	return t2_actuator_fire((u8)waveform_id, 2);
}

static const struct kernel_param_ops t2_actuator_test_fire_ops = {
	.set = t2_actuator_test_fire_set,
	.get = NULL,
};

module_param_cb(test_fire, &t2_actuator_test_fire_ops, NULL, 0200);
MODULE_PARM_DESC(test_fire,
	"write 1 (real captured click) or 9 (diagnostic probe) to fire that waveform ID at firm strength without a real click");

static bool t2_trackpad_actuator_matches(struct hid_device *hdev)
{
	/* Match the application collection. */
	unsigned int i;

	for (i = 0; i < hdev->maxcollection; i++) {
		if (hdev->collection[i].type == HID_COLLECTION_APPLICATION &&
		    hdev->collection[i].usage == T2_ACTUATOR_COLLECTION_USAGE)
			return true;
	}

	return false;
}

static int t2_trackpad_actuator_probe(struct hid_device *hdev,
				      const struct hid_device_id *id)
{
	struct t2_actuator *act;
	int ret;

	ret = hid_parse(hdev);
	if (ret)
		return ret;

	if (!t2_trackpad_actuator_matches(hdev))
		return -ENODEV;

	act = devm_kzalloc(&hdev->dev, sizeof(*act), GFP_KERNEL);
	if (!act)
		return -ENOMEM;

	act->hdev = hdev;
	hid_set_drvdata(hdev, act);

	ret = hid_hw_start(hdev, HID_CONNECT_HIDRAW);
	if (ret)
		return ret;

	mutex_lock(&t2_actuator_lock);
	t2_actuator_active = act;
	mutex_unlock(&t2_actuator_lock);

	hid_info(hdev, "actuator interface bound\n");
	return 0;
}

static void t2_trackpad_actuator_remove(struct hid_device *hdev)
{
	struct t2_actuator *act = hid_get_drvdata(hdev);

	mutex_lock(&t2_actuator_lock);
	if (t2_actuator_active == act)
		t2_actuator_active = NULL;
	mutex_unlock(&t2_actuator_lock);

	hid_hw_stop(hdev);
}

#define T2_ACTUATOR_DEVICE(model) \
	{ HID_USB_DEVICE(USB_VENDOR_ID_APPLE, \
		USB_DEVICE_ID_APPLE_WELLSPRINGT2_##model) }

static const struct hid_device_id t2_trackpad_actuator_devices[] = {
	T2_ACTUATOR_DEVICE(J140K),
	T2_ACTUATOR_DEVICE(J132),
	T2_ACTUATOR_DEVICE(J680),
	T2_ACTUATOR_DEVICE(J680_ALT),
	T2_ACTUATOR_DEVICE(J213),
	T2_ACTUATOR_DEVICE(J214K),
	T2_ACTUATOR_DEVICE(J223),
	T2_ACTUATOR_DEVICE(J230K),
	T2_ACTUATOR_DEVICE(J152F),
	{ }
};
MODULE_DEVICE_TABLE(hid, t2_trackpad_actuator_devices);

static struct hid_driver t2_trackpad_actuator_driver = {
	.name = "t2_trackpad_actuator",
	.id_table = t2_trackpad_actuator_devices,
	.probe = t2_trackpad_actuator_probe,
	.remove = t2_trackpad_actuator_remove,
};
module_hid_driver(t2_trackpad_actuator_driver);

MODULE_AUTHOR("André Eikmeyer <andre.eikmeyer@kait2en.org>");
MODULE_DESCRIPTION("Apple T2 trackpad Taptic Engine actuator driver");
MODULE_LICENSE("GPL");
MODULE_VERSION("0.01");
