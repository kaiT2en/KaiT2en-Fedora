// SPDX-License-Identifier: GPL-2.0-only
/*
 * Driver for Apple T2 MacBook internal trackpads, reverse engineered
 * from Tahoe/BridgeOS as 1:1 as possible. Aiming for precise pointer
 * behaviour as known from MacOS. We bought a Bentley, not Ford, right?
 *
 *
 * Implemented: T2 reports, firmware geometry, path IDs, scalar Z proximity,
 * Force Touch/Click, and the strongest-path hysteretic force filter.
 *
 * Remaining: parts of contactZShape, diving-button, and resting-hand paths.
 * The latter is clearly libinput territory.
 *
 * Althouth I did not use any of their code, let's credit T2Linux for their
 * pioneer work, which gave us a reliable upstream driver in the last years.
 *
 *
 *
 * André Eikmeyer, September 2026
 */

#define pr_fmt(fmt) KBUILD_MODNAME ": " fmt

#include <linux/hid.h>
#include <linux/input/mt.h>
#include <linux/ktime.h>
#include <linux/module.h>
#include <linux/mutex.h>
#include <linux/slab.h>

#include "hid-ids.h"

/*
 * Weak link to t2_trackpad_actuator: symbol_get()/symbol_put() instead of a
 * direct call, so this driver builds, loads and works with or without that
 * module present. The actuator is a separate, potentially upstreamable
 * module and must not become a hard dependency of this downstream one.
 */
typedef int (*t2_actuator_fire_fn)(u8 waveform_id, u8 strength);

extern int t2_actuator_fire(u8 waveform_id, u8 strength);

/*
 * t2_trackpad_actuator's waveform_id 1: plays back a DTrace-captured
 * click report body.
 */
#define T2_ACTUATOR_WAVEFORM_CLICK	1

static void t2_trackpad_fire_actuator(struct hid_device *hdev, u8 waveform_id, u8 strength)
{
	t2_actuator_fire_fn fire = (t2_actuator_fire_fn)symbol_get(t2_actuator_fire);
	int ret;

	if (!fire) {
		hid_warn(hdev, "actuator fire skipped: t2_actuator_fire not resolved (module not loaded/bound?)\n");
		return;
	}
	ret = fire(waveform_id, strength);
	if (ret)
		hid_warn(hdev, "actuator fire failed: %d\n", ret);
	symbol_put(t2_actuator_fire);
}

#define T2_TRACKPAD_REPORT_ID		0x02
#define T2_SURFACE_DIMENSIONS_REPORT_ID	0xd9
#define T2_MAX_CONTACTS			16
#define T2_MAX_ORIENTATION		16384
#define T2_DIMENSIONS_SIZE		17
/*
 * path_id_and_state's low byte is a persistent firmware path identity.
 * The high byte is a contact lifecycle state (observed: 1/2/3 while a
 * settling contact's proximity grows, 4 while stably touching, 5/7 while
 * lifting off, 0 once gone). A 10-bit mask reaches into the state byte and
 * makes every state transition look like a new path. Confirmed against a
 * capture where 0x107 -> 0x207 -> 0x307 -> 0x407 was a single physical
 * contact settling.
 */
#define T2_CONTACT_PATH_ID_MASK		0x00ff

#define T2_FORCE_Q16_ONE		65536
#define T2_FORCE_HYSTERESIS_SCALE_Q16	1311 /* Tahoe: 0.02 */
#define T2_FORCE_HYSTERESIS_MIN_Q16	98304 /* Tahoe: 1.5 */
#define T2_FORCE_HYSTERESIS_MAX_Q16	983040 /* Tahoe: 15.0 */
#define T2_FORCE_HYSTERESIS_ALPHA_Q16	9830 /* Tahoe: 0.15 */
/* MTForceFilter's this+0xc, from MTForceFilter::MTForceFilter's field init (0x3f75c28f = 0.96f). */
#define T2_FORCE_OUTPUT_ALPHA_Q16	62915 /* Tahoe: 0.96 */
/* Tahoe's default (medium-strength) MTForceConfig for force-capable trackpads. */
#define T2_FORCE_ACTIVATION_BASE_Q16	(125 * T2_FORCE_Q16_ONE)
#define T2_FORCE_RELEASE_BASE_Q16	(93 * T2_FORCE_Q16_ONE + 49152) /* 93.75 */
/* Firmware sparse-frame limit for per-contact force state. */
#define T2_FORCE_MISSING_FRAME_TOLERANCE	3

/*
 * MTForceThresholding::getClickThresholdMultiplier's real table (0x43c040,
 * data at 0x44ab9c): light/medium/firm scale the base thresholds by
 * 0.8/1.0/1.2. This is macOS's own "Click" pressure setting.
 */
#define T2_CLICK_STRENGTH_LIGHT_Q16	52429  /* 0.8 */
#define T2_CLICK_STRENGTH_MEDIUM_Q16	65536  /* 1.0 */
#define T2_CLICK_STRENGTH_FIRM_Q16	78643  /* 1.2 */

static unsigned int click_strength = 1;
module_param(click_strength, uint, 0644);
MODULE_PARM_DESC(click_strength,
	"Force Touch click pressure: 0=light, 1=medium (default), 2=firm");

static u32 t2_trackpad_click_strength_multiplier_q16(void)
{
	switch (click_strength) {
	case 0:
		return T2_CLICK_STRENGTH_LIGHT_Q16;
	case 2:
		return T2_CLICK_STRENGTH_FIRM_Q16;
	default:
		return T2_CLICK_STRENGTH_MEDIUM_Q16;
	}
}

/*
 * BTN_MOUSE fires at the plain click threshold above. A real force click
 * (the actuator/haptic feedback event)is a second, harder press on top of
 * that. Without this, an ordinary click almost always also crossed the 
 * single threshold. It is tunable since there is no confirmed real 
 * second-tier constant.
 */
static unsigned int force_click_threshold_percent = 175;
module_param(force_click_threshold_percent, uint, 0644);
MODULE_PARM_DESC(force_click_threshold_percent,
	"force click (actuator) threshold as a percent of the plain click threshold (default 175)");

/* Bounded raw-frame observation for Force Touch reconstruction. */
static unsigned int force_trace_frames;
module_param(force_trace_frames, uint, 0644);
MODULE_PARM_DESC(force_trace_frames,
	"number of T2 touch frames to log with force reconstruction inputs");

static unsigned int force_trace_trigger_pressure = 64;
module_param(force_trace_trigger_pressure, uint, 0644);
MODULE_PARM_DESC(force_trace_trigger_pressure,
	"raw pressure that arms the bounded Force Touch observation trace");

/*
 * Reconstruction of position-hysteresis
 * (MTParserPath::pullHysteresisCenterHidingUnstablePixelDeltasXY, 0x40f55c):
 * position moves toward the raw contact by at most a per-frame budget,
 * budget = instability * scale. touchTimeDebounce and the Z-signal delta
 * portion of contactZShape are reconstructed. Their constants are from
 * MTParameterFactory::initPathFilterParams (0x434a10) and the Tahoe
 * MTPathFilterParameters observed on a MacBookPro15,1, respectively.
 * The remaining terms require further reconstruction:
 *
 * - contactZShape's sliding and large-ellipse branches:
 *   MTParserPath::computeZSignalInstability. They need Tahoe's parser-state
 *   transition, not an invented motion threshold.
 * - divingButtonZone: MTParserPath::computeDivingButtonChangeInstability.
 *   depends on macOS's button-zone state.
 * - restingHand: MTParserPath::isStuckOnDivingRegion plus its hand state.
 * - buttonTimeDebounce: the button-stage timing path in measureInstability.
 *
 */
#define T2_INSTABILITY_STRONG_MS	50
#define T2_INSTABILITY_WEAK_MS		312
#define T2_Q16_ONE			65536

/* Tahoe MTPathFilterParameters at computeZSignalInstability (Q16). */
#define T2_Z_DELTA_PERCENT_LOW_Q16	(4 * T2_Q16_ONE)
#define T2_Z_DELTA_PERCENT_HIGH_Q16	(8 * T2_Q16_ONE)

static unsigned int hysteresis_budget_max = 512;
module_param(hysteresis_budget_max, uint, 0644);
MODULE_PARM_DESC(hysteresis_budget_max,
	"pointer step budget (raw position units) at instability=1.0; our own bridge constant, not Tahoe's");

/*
 * The disassembly reads as budget growing *with* instability, which
 * contradicts the function's own name and could not be resolved by
 * re-checking the SIMD. Non-inverted, budget decays to 0 and stays there,
 * permanently freezing position. Inverted is the
 * only polarity that does not lock up, so this should be it.
 */
static bool position_hysteresis_invert = true;
module_param(position_hysteresis_invert, bool, 0644);
MODULE_PARM_DESC(position_hysteresis_invert,
	"budget grows as instability decays instead of shrinking with it");

/*
 * BTN_LEFT is the reconstructed Force Touch state machine's output
 * (per-path hysteretic force, strongest-path locking), not the raw firmware
 * buttons bit, which has no multi-contact analysis at all.
 */

struct t2_trackpad_finger {
	/* Firmware path ID in the low byte. The high byte is contact state. */
	__le16 path_id_and_state;
	__le16 unknown2;
	__le16 abs_x;
	__le16 abs_y;
	__le16 rel_x;
	__le16 rel_y;
	__le16 tool_major;
	__le16 tool_minor;
	__le16 orientation;
	/* MTContact.proximity, unsigned Q8 (confirmed with Tahoe FBT). */
	__le16 proximity_q8;
	__le16 touch_minor;
	__le16 unused[2];
	__le16 pressure;
	__le16 multi;
} __packed __aligned(2);

struct t2_trackpad_frame {
	u8 unknown[22];
	u8 num_fingers;
	u8 buttons;
	u8 unknown2[14];
	struct t2_trackpad_finger fingers[];
} __packed;

struct t2_trackpad_mouse_report {
	u8 report_id;
	u8 buttons;
	u8 rel_x;
	u8 rel_y;
	u8 padding[4];
} __packed;

struct t2_surface_dimensions {
	/* Physical surface size in 1/100 mm. */
	__le32 width;
	__le32 height;
	/* Signed raw-coordinate endpoints for the report-0x02 contacts. */
	__le16 min_x;
	__le16 min_y;
	__le16 max_x;
	__le16 max_y;
} __packed;

struct t2_trackpad_geometry {
	u32 width;
	u32 height;
	int min_x;
	int min_y;
	int max_x;
	int max_y;
	unsigned int x_res;
	unsigned int y_res;
};

struct t2_mt_force_filter {
	s32 hysteretic_force_q16;
	/* MTForceFilter's this+8: the asymmetrically-smoothed final output. */
	s32 output_q16;
	/* MTForceFilter's this+0x10: raw pressure smoothed before hysteresis. */
	s32 smoothed_input_q16;
};

struct t2_force_path {
	struct t2_mt_force_filter filter;
	bool present;
	u8 missing_frames;
};

struct t2_force_management {
	struct t2_force_path paths[T2_MAX_CONTACTS];
	bool button_activated;
	bool force_click_activated;
};

/* Per-slot hysteresis center and its Tahoe-derived Z history, keyed by path_id. */
struct t2_slot_hysteresis {
	s32 x;
	s32 y;
	u64 touch_start_ns;
	u16 path_id;
	/* T2 proximity_q8 is MTContact.proximity, not a reported input axis. */
	u16 previous_proximity_q8;
	/* Set only while this firmware path is present in the current report. */
	bool present_this_frame;
	bool valid;
};

struct t2_trackpad {
	struct hid_device *hdev;
	struct input_dev *input;
	/*
	 * Separate device for BTN_TASK. Adding it to input above broke
	 * click/drag on hardware. libinput picks its click method from a
	 * touchpad's BTN_* set, an extra code confused it into misreporting
	 * clicks as double-clicks and dropping drags.
	 */
	struct input_dev *event_input;
	struct t2_trackpad_geometry geometry;
	struct mutex geometry_lock;
	struct t2_force_management force;
	struct t2_slot_hysteresis hysteresis[T2_MAX_CONTACTS];
	u64 previous_frame_ns;
	bool force_trace_active;
	bool dimensions_ready;
};

static inline int t2_s16(__le16 value)
{
	return (s16)le16_to_cpu(value);
}

static u16 t2_trackpad_ellipse_axis(__le16 radius)
{
	/* Firmware shape values are ellipse radii. The MT ABI uses full axes. */
	return min_t(int, max_t(int, t2_s16(radius), 0) * 2, U16_MAX);
}

static s32 t2_mt_update_hysteretic_force(struct t2_mt_force_filter *filter,
					  u16 force)
{
	s32 hysteresis_q16;
	s32 candidate_q16;
	s64 filtered_q16;
	s32 raw_q16;

	/*
	 * MTForceFilter::updateForceFilter (0x43b94a), first stage: raw
	 * pressure is smoothed before hysteresis, not fed in directly. We
	 * do not have the real velocity-gated alpha choice, so this reuses
	 * the one confirmed alpha (0.15/0.85). Without it, single-frame
	 * sensor noise right at a threshold crossing caused a real,
	 * few-ms spurious BTN_MOUSE release immediately after activation.
	 */
	raw_q16 = min_t(u16, force, S32_MAX / T2_FORCE_Q16_ONE) * T2_FORCE_Q16_ONE;
	filter->smoothed_input_q16 = (s32)(((s64)T2_FORCE_HYSTERESIS_ALPHA_Q16 *
		filter->smoothed_input_q16 +
		(s64)(T2_FORCE_Q16_ONE - T2_FORCE_HYSTERESIS_ALPHA_Q16) * raw_q16) /
		T2_FORCE_Q16_ONE);

	hysteresis_q16 = force * T2_FORCE_HYSTERESIS_SCALE_Q16;
	hysteresis_q16 = clamp(hysteresis_q16, T2_FORCE_HYSTERESIS_MIN_Q16,
				       T2_FORCE_HYSTERESIS_MAX_Q16);
	candidate_q16 = filter->smoothed_input_q16;
	if (candidate_q16 > filter->hysteretic_force_q16 + hysteresis_q16)
		candidate_q16 -= hysteresis_q16;
	else if (candidate_q16 < filter->hysteretic_force_q16 - hysteresis_q16)
		candidate_q16 += hysteresis_q16;
	else
		candidate_q16 = filter->hysteretic_force_q16;

	/*
	 * MTForceFilter::updateHystereticForce (0x43ba88). Alpha weights the
	 * old value (0.15 old, 0.85 new). Fast tracking of the candidate.
	 * Confirmed from the disassembly.
	 */
	filtered_q16 = (s64)T2_FORCE_HYSTERESIS_ALPHA_Q16 *
		filter->hysteretic_force_q16 +
		(s64)(T2_FORCE_Q16_ONE - T2_FORCE_HYSTERESIS_ALPHA_Q16) *
		candidate_q16;
	filter->hysteretic_force_q16 = filtered_q16 / T2_FORCE_Q16_ONE;

	/*
	 * MTForceFilter::updateForceFilter (0x43b94a), second stage.
	 * Asymmetric: rising is damped 0.96 old/0.04 new, falling is instant.
	 * This is a separate, delayed filter output. Tahoe's force-stage
	 * classification uses the hysteretic stage above, rather than this value.
	 */
	if (filter->hysteretic_force_q16 > filter->output_q16) {
		s64 output_q16 = (s64)T2_FORCE_OUTPUT_ALPHA_Q16 * filter->output_q16 +
			(s64)(T2_FORCE_Q16_ONE - T2_FORCE_OUTPUT_ALPHA_Q16) *
			filter->hysteretic_force_q16;
		filter->output_q16 = output_q16 / T2_FORCE_Q16_ONE;
	} else {
		filter->output_q16 = filter->hysteretic_force_q16;
	}
	return filter->output_q16;
}

/* Marks the slot present this frame and feeds its pressure into the filter. */
static void t2_trackpad_forward_firmware_distributed_forces(
		struct t2_trackpad *tp, int slot,
		const struct t2_trackpad_finger *finger)
{
	struct t2_force_path *path = &tp->force.paths[slot];

	path->present = true;
	path->missing_frames = 0;
	t2_mt_update_hysteretic_force(&path->filter,
				       max_t(int, t2_s16(finger->pressure), 0));
}

static bool t2_trackpad_is_force_button_activated(
		const struct t2_force_management *force)
{
	return force->button_activated;
}

static void t2_trackpad_analyze_and_manage_strongest_forces(
		struct t2_trackpad *tp)
{
	struct t2_force_management *force = &tp->force;
	int strongest_slot = -1;
	s32 strongest_force = 0;
	u32 multiplier_q16 = t2_trackpad_click_strength_multiplier_q16();
	s32 activation_q16 = (s32)(((s64)T2_FORCE_ACTIVATION_BASE_Q16 * multiplier_q16) /
				    T2_FORCE_Q16_ONE);
	s32 release_q16 = (s32)(((s64)T2_FORCE_RELEASE_BASE_Q16 * multiplier_q16) /
				 T2_FORCE_Q16_ONE);
	s32 fc_activation_q16 = (s32)(((s64)activation_q16 * force_click_threshold_percent) / 100);
	s32 fc_release_q16 = (s32)(((s64)release_q16 * force_click_threshold_percent) / 100);
	int i;

	/*
	 * MTForceManagement::isForceButtonActivated uses the strongest active
	 * MTForceFilter hysteretic value. MTParserPath::measureInstability
	 * buttonTimeDebounce remains pending in the position path.
	 *
	 * Tahoe locks the strongest progress path once activation occurs.
	 * A path missing for only a few frames still counts, matching the
	 * filter-state tolerance in t2_trackpad_finish_force_frame.
	 */
	for (i = 0; i < T2_MAX_CONTACTS; i++) {
		if (!force->paths[i].present &&
		    force->paths[i].missing_frames >= T2_FORCE_MISSING_FRAME_TOLERANCE)
			continue;
		/*
		 * Tahoe's force thresholding consumes MTForceFilter's hysteretic
		 * value (this+0x28), not its delayed output (this+0x08). Using the
		 * latter made a short physical click disappear before the 0.04
		 * rising filter could cross the threshold.
		 */
		if (strongest_slot < 0 ||
		    force->paths[i].filter.hysteretic_force_q16 > strongest_force) {
			strongest_slot = i;
			strongest_force = force->paths[i].filter.hysteretic_force_q16;
		}
	}

	if (!force->button_activated) {
		if (strongest_slot >= 0 && strongest_force >= activation_q16) {
			force->button_activated = true;
		}
	} else if (strongest_slot < 0 || strongest_force < release_q16) {
		/*
		 * Release on the strongest *currently* present contact, not a
		 * slot pinned at activation time. The firmware occasionally
		 * assigns a new path_id to the same still-pressing finger.
		 * Recomputing strongest_force here the same way activation
		 * does is immune to that.
		 */
		force->button_activated = false;
	}

	if (!force->force_click_activated) {
		if (strongest_slot >= 0 && strongest_force >= fc_activation_q16) {
			force->force_click_activated = true;
			t2_trackpad_fire_actuator(tp->hdev, T2_ACTUATOR_WAVEFORM_CLICK,
						  click_strength);
		}
	} else if (strongest_slot < 0 || strongest_force < fc_release_q16) {
		force->force_click_activated = false;
	}
}

static bool t2_trackpad_finish_force_frame(struct t2_trackpad *tp)
{
	bool button_activated;
	int i;

	/*
	 * The current report's presence map must remain intact through Tahoe's
	 * strongest-path decision. raw_event() starts the following report with
	 * a fresh map, so only filters for contacts missing from this report are
	 * discarded here.
	 */
	t2_trackpad_analyze_and_manage_strongest_forces(tp);
	button_activated = t2_trackpad_is_force_button_activated(&tp->force);

	/*
	 * A single sparse frame (proximity_q8=0 for one sample on a still
	 * pressing finger) retains the force filter for a bounded number of
	 * missing frames.
	 */
	for (i = 0; i < T2_MAX_CONTACTS; i++) {
		if (tp->force.paths[i].present)
			continue;
		if (++tp->force.paths[i].missing_frames >= T2_FORCE_MISSING_FRAME_TOLERANCE) {
			tp->force.paths[i].filter.hysteretic_force_q16 = 0;
			tp->force.paths[i].filter.output_q16 = 0;
			tp->force.paths[i].filter.smoothed_input_q16 = 0;
		}
	}

	return button_activated;
}

static int t2_trackpad_enable_multitouch(struct hid_device *hdev)
{
	const u8 feature[] = { T2_TRACKPAD_REPORT_ID, 0x01 };
	u8 *buf;
	int ret;

	/*
	 * Report 0x02 starts with the conventional mouse header and is followed
	 * by the T2 vendor multitouch frame. Setting byte 1 enables that vendor
	 * frame. Without it the controller only emits the mouse portion.
	 *
	 * usbhid submits this buffer as a USB control URB. In particular, the
	 * T2 virtual HCD rejects stack-backed transfer buffers, so the request
	 * must use DMA-mappable heap storage.
	 */
	buf = kmemdup(feature, sizeof(feature), GFP_KERNEL);
	if (!buf)
		return -ENOMEM;

	ret = hid_hw_raw_request(hdev, buf[0], buf, sizeof(feature),
				 HID_FEATURE_REPORT, HID_REQ_SET_REPORT);
	kfree(buf);
	return ret;
}

static int t2_trackpad_get_geometry(struct t2_trackpad *tp,
				    struct t2_trackpad_geometry *geometry)
{
	struct t2_surface_dimensions dimensions;
	u8 *buf;
	int x_span, y_span;
	int ret;

	/*
	 * macOS's AppleMultitouchHidPlugin first asks its MTDevice for
	 * MTDeviceGetParserType() and MTDeviceGetParserOptions().  Its
	 * MultitouchHIDClass startup dispatches type 1000 to
	 * MTTrackpadHIDManager and type 1001 to MTTrackpadEmbeddedHIDManager:
	 * these select a parser/transport path, not a model-specific calibration
	 * profile.  Keep that distinction when migrating another hid_t2magicmouse
	 * device: identify its report path before adding support.
	 *
	 * Vendor GET_FEATURE 0xd9 is not represented in the ordinary HID input
	 * descriptor. It returns the report ID, little-endian width and height in
	 * 1/100 mm, then signed coordinate endpoints. MTDevice exposes the same
	 * sensor surface to AppleMultitouchHidPlugin.
	 *
	 * Resolution is 100 * coordinate span / surface length in raw units per
	 * millimetre. Geometry belongs to the attached T2 device.
	 */
	buf = kzalloc(T2_DIMENSIONS_SIZE, GFP_KERNEL);
	if (!buf)
		return -ENOMEM;
	buf[0] = T2_SURFACE_DIMENSIONS_REPORT_ID;

	ret = hid_hw_raw_request(tp->hdev, T2_SURFACE_DIMENSIONS_REPORT_ID, buf,
				 T2_DIMENSIONS_SIZE, HID_FEATURE_REPORT,
				 HID_REQ_GET_REPORT);
	if (ret < 1 + sizeof(dimensions)) {
		hid_err(tp->hdev, "invalid surface dimensions report (%d)\n", ret);
		ret = ret < 0 ? ret : -EPROTO;
		goto out;
	}

	memcpy(&dimensions, buf + 1, sizeof(dimensions));
	geometry->width = le32_to_cpu(dimensions.width);
	geometry->height = le32_to_cpu(dimensions.height);
	geometry->min_x = t2_s16(dimensions.min_x);
	geometry->min_y = t2_s16(dimensions.min_y);
	geometry->max_x = t2_s16(dimensions.max_x);
	geometry->max_y = t2_s16(dimensions.max_y);
	x_span = geometry->max_x - geometry->min_x;
	y_span = geometry->max_y - geometry->min_y;
	if (!geometry->width || !geometry->height || x_span <= 0 || y_span <= 0) {
		hid_err(tp->hdev, "invalid surface dimensions\n");
		ret = -EPROTO;
		goto out;
	}

	/* width/height are 1/100 mm, so this yields raw coordinate units per mm. */
	geometry->x_res = 100 * x_span / geometry->width;
	geometry->y_res = 100 * y_span / geometry->height;
	ret = 0;
out:
	kfree(buf);
	return ret;
}

static void t2_trackpad_apply_geometry(struct input_dev *input,
				       const struct t2_trackpad_geometry *geometry)
{
	/*
	 * I am an Idiot! T2 Y grows opposite to Linux input coordinates.
	 */
	input_set_abs_params(input, ABS_MT_POSITION_X, geometry->min_x,
			     geometry->max_x, 0, 0);
	input_set_abs_params(input, ABS_MT_POSITION_Y, -geometry->max_y,
			     -geometry->min_y, 0, 0);
	input_set_abs_params(input, ABS_X, geometry->min_x, geometry->max_x, 0, 0);
	input_set_abs_params(input, ABS_Y, -geometry->max_y, -geometry->min_y, 0, 0);
	input_abs_set_res(input, ABS_MT_POSITION_X, geometry->x_res);
	input_abs_set_res(input, ABS_MT_POSITION_Y, geometry->y_res);
	input_abs_set_res(input, ABS_X, geometry->x_res);
	input_abs_set_res(input, ABS_Y, geometry->y_res);
}

static int t2_trackpad_query_dimensions(struct t2_trackpad *tp)
{
	struct t2_trackpad_geometry geometry;
	int ret;

	ret = t2_trackpad_get_geometry(tp, &geometry);
	if (ret)
		return ret;

	/* The input open path completes this before returning the event node. */
	tp->geometry = geometry;
	t2_trackpad_apply_geometry(tp->input, &tp->geometry);
	hid_info(tp->hdev, "surface %u x %u, coordinates %d..%d x %d..%d\n",
		 tp->geometry.width, tp->geometry.height, tp->geometry.min_x,
		 tp->geometry.max_x, tp->geometry.min_y, tp->geometry.max_y);
	return 0;
}

static int t2_trackpad_open(struct input_dev *input)
{
	struct hid_device *hdev = input_get_drvdata(input);
	struct t2_trackpad *tp = hid_get_drvdata(hdev);
	int ret;

	ret = hid_hw_open(hdev);
	if (ret)
		return ret;

	ret = t2_trackpad_enable_multitouch(hdev);
	if (ret < 0) {
		hid_err(hdev, "could not enable multitouch (%d)\n", ret);
		goto err_close;
	}
	ret = 0;

	mutex_lock(&tp->geometry_lock);
	if (!tp->dimensions_ready) {
		ret = t2_trackpad_query_dimensions(tp);
		if (!ret)
			tp->dimensions_ready = true;
	}
	mutex_unlock(&tp->geometry_lock);
	if (ret)
		goto err_close;

	return 0;

err_close:
	hid_hw_close(hdev);
	return ret;
}

static void t2_trackpad_close(struct input_dev *input)
{
	hid_hw_close(input_get_drvdata(input));
}

static u32 t2_trackpad_touch_time_instability_q16(u64 elapsed_ns)
{
	/* MTParserPath::measureInstability touchTimeDebounce term. */
	u32 elapsed_ms = elapsed_ns / NSEC_PER_MSEC;
	u32 strong = 0, weak = 0;

	if (elapsed_ms < T2_INSTABILITY_STRONG_MS)
		strong = (u32)T2_Q16_ONE * (T2_INSTABILITY_STRONG_MS - elapsed_ms) /
			 T2_INSTABILITY_STRONG_MS;
	if (elapsed_ms < T2_INSTABILITY_WEAK_MS)
		weak = (u32)T2_Q16_ONE * (T2_INSTABILITY_WEAK_MS - elapsed_ms) /
		       T2_INSTABILITY_WEAK_MS / 5;

	return max(strong, weak);
}

static u32 t2_trackpad_z_signal_instability_q16(
		struct t2_slot_hysteresis *hys, u16 proximity_q8)
{
	u32 denominator_q8;
	u32 delta_percent_q16;
	u32 linear_q16;
	u64 square_q16;
	u64 cube_q32;

	/*
	 * MTParserPath::computeZSignalInstability reads MTContact.proximity
	 * (this+0x48) and its preceding value (this+0xa8). FBT on Tahoe proved
	 * that T2's proximity_q8 supplies that value: 0x008d / 256.0 is
	 * Tahoe's observed 0.55078 proximity. pressure is Force Touch instead.
	 *
	 * Tahoe's disassembly forms abs(delta) * 100 / max(current, previous),
	 * maps its live MTPathFilterParameters window 4.0..8.0 to 0..1, then
	 * raises it to 1.5. sqrt(x^3) is our fixed-point reconstruction of that
	 * scalar branch. It is not evidence for the still-unimplemented branches.
	 */
	denominator_q8 = max_t(u16, proximity_q8, hys->previous_proximity_q8);
	if (!denominator_q8)
		return 0;

	delta_percent_q16 = div_u64((u64)abs((int)proximity_q8 -
						   hys->previous_proximity_q8) *
					   100 * T2_Q16_ONE, denominator_q8);
	if (delta_percent_q16 <= T2_Z_DELTA_PERCENT_LOW_Q16)
		return 0;
	if (delta_percent_q16 >= T2_Z_DELTA_PERCENT_HIGH_Q16)
		return T2_Q16_ONE;

	linear_q16 = div_u64((u64)(delta_percent_q16 -
					T2_Z_DELTA_PERCENT_LOW_Q16) * T2_Q16_ONE,
				  T2_Z_DELTA_PERCENT_HIGH_Q16 -
					T2_Z_DELTA_PERCENT_LOW_Q16);
	square_q16 = div_u64((u64)linear_q16 * linear_q16, T2_Q16_ONE);
	cube_q32 = square_q16 * linear_q16;
	return int_sqrt64(cube_q32);
}

static void t2_trackpad_report_finger(struct t2_trackpad *tp, int slot,
				       const struct t2_trackpad_finger *finger)
{
	struct t2_slot_hysteresis *hys = &tp->hysteresis[slot];
	struct input_dev *input = tp->input;
	u16 path_id = le16_to_cpu(finger->path_id_and_state) & T2_CONTACT_PATH_ID_MASK;
	s32 raw_x = t2_s16(finger->abs_x);
	s32 raw_y = t2_s16(finger->abs_y);
	u16 proximity_q8 = max_t(int, t2_s16(finger->proximity_q8), 0);
	u64 now = ktime_get_ns();
	u32 instability_q16;
	u32 z_instability_q16;
	s32 budget;
	s32 x, y;

	/* MTParserPath state is reset at a new firmware contact lifetime. */
	if (!hys->valid || hys->path_id != path_id) {
		hys->valid = true;
		hys->path_id = path_id;
		hys->touch_start_ns = now;
		hys->x = raw_x;
		hys->y = raw_y;
		hys->previous_proximity_q8 = proximity_q8;
	}

	instability_q16 = t2_trackpad_touch_time_instability_q16(now - hys->touch_start_ns);
	z_instability_q16 = t2_trackpad_z_signal_instability_q16(hys,
								 proximity_q8);
	instability_q16 = max(instability_q16, z_instability_q16);
	hys->previous_proximity_q8 = proximity_q8;
	budget = (s32)((u64)instability_q16 * hysteresis_budget_max / T2_Q16_ONE);
	if (position_hysteresis_invert)
		budget = hysteresis_budget_max - budget;

	x = clamp_t(s32, raw_x, hys->x - budget, hys->x + budget);
	y = clamp_t(s32, raw_y, hys->y - budget, hys->y + budget);
	hys->x = x;
	hys->y = y;
	hys->present_this_frame = true;

	input_mt_slot(input, slot);
	input_mt_report_slot_state(input, MT_TOOL_FINGER, true);
	/*
	 * Keep the established Linux touch/width split for libinput: the firmware's
	 * Q8 proximity channel remains the touch-size signal while tool_major/minor
	 * describe the enclosing width ellipse below.  Tahoe uses the latter as
	 * MTContact radii internally.
	 */
	input_report_abs(input, ABS_MT_TOUCH_MAJOR,
			 t2_trackpad_ellipse_axis(finger->proximity_q8));
	input_report_abs(input, ABS_MT_TOUCH_MINOR,
			 t2_trackpad_ellipse_axis(finger->touch_minor));
	input_report_abs(input, ABS_MT_WIDTH_MAJOR,
			 t2_trackpad_ellipse_axis(finger->tool_major));
	input_report_abs(input, ABS_MT_WIDTH_MINOR,
			 t2_trackpad_ellipse_axis(finger->tool_minor));
	/* Firmware orientation runs opposite the MT ABI's convention. */
	input_report_abs(input, ABS_MT_ORIENTATION,
			 T2_MAX_ORIENTATION - t2_s16(finger->orientation));
	input_report_abs(input, ABS_MT_POSITION_X, x);
	input_report_abs(input, ABS_MT_POSITION_Y, -y);
}

static int t2_trackpad_path_slot(struct input_dev *input,
				 const struct t2_trackpad_finger *finger)
{
	/*
	 * The first T2 word evolves 0x4xx -> 0x5xx -> 0x7xx -> 0x0xx over
	 * liftoff while xx remains stable. That stable portion is the persistent
	 * firmware path identity required by Tahoe's MTPathStates. It replaces
	 * nearest-position assignment, which can exchange paths during a drag.
	 */
	u16 path_id = le16_to_cpu(finger->path_id_and_state) &
		      T2_CONTACT_PATH_ID_MASK;

	return input_mt_get_slot_by_key(input, path_id);
}

static void t2_trackpad_trace_force_frame(struct t2_trackpad *tp,
					 const struct t2_trackpad_frame *frame)
{
	unsigned int remaining;
	u64 now, elapsed_us;
	int i;

	/*
	 * Tahoe's MTPathStates::forwardFirmwareDistributedForces uses per-path
	 * firmware force, velocity and frame interval.On T2, pressure is the
	 * decoded per-contact candidate for that force.The other raw fields are
	 * logged only to establish path identity and click-source rules from
	 * captures.
	 */
	remaining = READ_ONCE(force_trace_frames);
	if (!remaining)
		return;
	if (!tp->force_trace_active) {
		for (i = 0; i < frame->num_fingers; i++) {
			if (t2_s16(frame->fingers[i].pressure) >=
			    force_trace_trigger_pressure) {
				tp->force_trace_active = true;
				tp->previous_frame_ns = 0;
				break;
			}
		}
		if (!tp->force_trace_active)
			return;
	}
	if (cmpxchg(&force_trace_frames, remaining, remaining - 1) != remaining)
		return;

	now = ktime_get_ns();
	elapsed_us = tp->previous_frame_ns ?
		(now - tp->previous_frame_ns) / NSEC_PER_USEC : 0;
	tp->previous_frame_ns = now;

	hid_info(tp->hdev, "force frame: dt=%lluus reported=%u buttons=%#x\n",
		 elapsed_us, frame->num_fingers, frame->buttons);
	for (i = 0; i < frame->num_fingers; i++) {
		const struct t2_trackpad_finger *finger = &frame->fingers[i];

		hid_info(tp->hdev,
			 "force contact[%d]: u1=%d u2=%d pos=%d,%d rel=%d,%d "
			 "proximity=%d touch_minor=%d tool=%d,%d pressure=%d multi=%d unused=%d,%d\n",
			 i, t2_s16(finger->path_id_and_state), t2_s16(finger->unknown2),
			 t2_s16(finger->abs_x), t2_s16(finger->abs_y),
			 t2_s16(finger->rel_x), t2_s16(finger->rel_y),
			 t2_s16(finger->proximity_q8), t2_s16(finger->touch_minor),
			 t2_s16(finger->tool_major), t2_s16(finger->tool_minor),
			 t2_s16(finger->pressure), t2_s16(finger->multi),
			 t2_s16(finger->unused[0]), t2_s16(finger->unused[1]));
	}
}

static int t2_trackpad_raw_event(struct hid_device *hdev,
				 struct hid_report *report, u8 *data, int size)
{
	struct t2_trackpad *tp = hid_get_drvdata(hdev);
	const struct t2_trackpad_frame *frame;
	const size_t mouse_size = sizeof(struct t2_trackpad_mouse_report);
	const size_t frame_size = sizeof(*frame);
	const size_t finger_size = sizeof(struct t2_trackpad_finger);
	bool force_button;
	int i;

	/* AppleMultitouchHIDEventDriverV2::handleInterruptReport receives this
	 * mouse prefix followed by the vendor contact frame. */
	if (size < mouse_size || data[0] != T2_TRACKPAD_REPORT_ID)
		return 0;

	data += mouse_size;
	size -= mouse_size;
	if (size < frame_size || (size - frame_size) % finger_size)
		return 0;

	frame = (const struct t2_trackpad_frame *)data;
	if (frame->num_fingers > T2_MAX_CONTACTS ||
	    (size - frame_size) / finger_size < frame->num_fingers) {
		hid_warn(hdev, "invalid contact count %u\n", frame->num_fingers);
		return 0;
	}
	t2_trackpad_trace_force_frame(tp, frame);
	/*
	 * A firmware path ID identifies a contact only for its current lifetime.
	 * input_mt_sync_frame() releases slots absent from this report, so discard
	 * the matching position-filter state as well.  Retaining it lets a later
	 * tap that reuses the path ID start from the prior tap's centre.
	 */
	for (i = 0; i < T2_MAX_CONTACTS; i++)
		tp->hysteresis[i].present_this_frame = false;
	for (i = 0; i < T2_MAX_CONTACTS; i++)
		tp->force.paths[i].present = false;

	for (i = 0; i < frame->num_fingers; i++) {
		const struct t2_trackpad_finger *finger = &frame->fingers[i];
		int slot;

		if (!t2_s16(finger->proximity_q8))
			continue;

		slot = t2_trackpad_path_slot(tp->input, finger);
		if (slot < 0) {
			hid_warn(hdev, "no MT slot for firmware path %u\n",
				 le16_to_cpu(finger->path_id_and_state) &
				 T2_CONTACT_PATH_ID_MASK);
			continue;
		}
		t2_trackpad_report_finger(tp, slot, finger);
		t2_trackpad_forward_firmware_distributed_forces(tp, slot, finger);
	}
	for (i = 0; i < T2_MAX_CONTACTS; i++) {
		if (tp->hysteresis[i].valid &&
		    !tp->hysteresis[i].present_this_frame) {
			tp->hysteresis[i].valid = false;
			tp->hysteresis[i].previous_proximity_q8 = 0;
		}
	}
	force_button = t2_trackpad_finish_force_frame(tp);

	input_mt_sync_frame(tp->input);
	input_report_key(tp->input, BTN_MOUSE, force_button);
	input_sync(tp->input);

	/*
	 * BTN_TASK is the force click, on a separate device (event_input).
	 * Lets third-party tools bind to it directly.
	 */
	input_report_key(tp->event_input, BTN_TASK, tp->force.force_click_activated);
	input_sync(tp->event_input);

	return 1;
}

static int t2_trackpad_event(struct hid_device *hdev, struct hid_field *field,
			     struct hid_usage *usage, __s32 value)
{
	/*
	 * Report 0x02 has a conventional mouse prefix followed by the vendor
	 * multitouch frame. t2_trackpad_raw_event() owns the complete report.
	 * letting hid-input process the prefix would independently emit its
	 * BTN_LEFT usage alongside the reconstructed Force Touch button.
	 */
	if (field->report->id == T2_TRACKPAD_REPORT_ID)
		return 1;

	return 0;
}

static int t2_trackpad_input_mapping(struct hid_device *hdev,
				     struct hid_input *hi, struct hid_field *field,
				     struct hid_usage *usage, unsigned long **bit, int *max)
{
	struct t2_trackpad *tp = hid_get_drvdata(hdev);

	if (!tp->input)
		tp->input = hi->input;

	return 0;
}

static int t2_trackpad_input_configured(struct hid_device *hdev,
					struct hid_input *hi)
{
	struct t2_trackpad *tp = hid_get_drvdata(hdev);
	struct input_dev *input = tp->input;
	int ret;

	if (!input)
		return -ENODEV;

	__set_bit(INPUT_PROP_BUTTONPAD, input->propbit);
	__clear_bit(EV_REL, input->evbit);
	__clear_bit(REL_X, input->relbit);
	__clear_bit(REL_Y, input->relbit);
	__clear_bit(BTN_0, input->keybit);
	__clear_bit(BTN_RIGHT, input->keybit);
	__clear_bit(BTN_MIDDLE, input->keybit);
	__clear_bit(EV_REP, input->evbit);
	input_set_capability(input, EV_KEY, BTN_MOUSE);

	__set_bit(EV_ABS, input->evbit);
	input_set_abs_params(input, ABS_MT_TOUCH_MAJOR, 0, U16_MAX, 0, 0);
	input_set_abs_params(input, ABS_MT_TOUCH_MINOR, 0, U16_MAX, 0, 0);
	input_set_abs_params(input, ABS_MT_WIDTH_MAJOR, 0, U16_MAX, 0, 0);
	input_set_abs_params(input, ABS_MT_WIDTH_MINOR, 0, U16_MAX, 0, 0);
	/*
	 * MTContact.force feeds MTForceFilter and is not advertised as
	 * ABS_MT_PRESSURE. libinput derives contact lifetime from MT slots.
	 */
	input_set_abs_params(input, ABS_MT_ORIENTATION, -T2_MAX_ORIENTATION,
			     T2_MAX_ORIENTATION, 0, 0);

	/* Placeholder bounds. t2_trackpad_query_dimensions() overwrites these. */
	input_set_abs_params(input, ABS_MT_POSITION_X, SHRT_MIN, SHRT_MAX, 0, 0);
	input_set_abs_params(input, ABS_MT_POSITION_Y, SHRT_MIN, SHRT_MAX, 0, 0);

	ret = input_mt_init_slots(input, T2_MAX_CONTACTS,
				  INPUT_MT_POINTER | INPUT_MT_DROP_UNUSED | INPUT_MT_TRACK);
	if (ret)
		return ret;

	input->open = t2_trackpad_open;
	input->close = t2_trackpad_close;
	input_set_events_per_packet(input, 60);
	return 0;
}

static int t2_trackpad_probe(struct hid_device *hdev,
			     const struct hid_device_id *id)
{
	struct t2_trackpad *tp;
	struct hid_report *report;
	int ret;

	if (hdev->type != HID_TYPE_USBMOUSE)
		return -ENODEV;

	tp = devm_kzalloc(&hdev->dev, sizeof(*tp), GFP_KERNEL);
	if (!tp)
		return -ENOMEM;

	tp->hdev = hdev;
	mutex_init(&tp->geometry_lock);
	hid_set_drvdata(hdev, tp);

	ret = hid_parse(hdev);
	if (ret)
		return ret;

	ret = hid_hw_start(hdev, HID_CONNECT_DEFAULT);
	if (ret)
		return ret;

	if (!tp->input) {
		hid_err(hdev, "no input device registered\n");
		ret = -ENODEV;
		goto err_stop;
	}

	tp->event_input = devm_input_allocate_device(&hdev->dev);
	if (!tp->event_input) {
		ret = -ENOMEM;
		goto err_stop;
	}
	tp->event_input->name = "T2 Force Click Events";
	tp->event_input->phys = hdev->phys;
	tp->event_input->uniq = hdev->uniq;
	tp->event_input->id.bustype = hdev->bus;
	tp->event_input->id.vendor = hdev->vendor;
	tp->event_input->id.product = hdev->product;
	tp->event_input->id.version = hdev->version;
	input_set_capability(tp->event_input, EV_KEY, BTN_TASK);
	ret = input_register_device(tp->event_input);
	if (ret)
		goto err_stop;

	report = hid_register_report(hdev, HID_INPUT_REPORT,
				     T2_TRACKPAD_REPORT_ID, 0);
	if (!report) {
		ret = -ENOMEM;
		goto err_stop;
	}
	report->size = 6;

	hid_info(hdev, "initialized\n");
	return 0;

err_stop:
	hid_hw_stop(hdev);
	return ret;
}

static void t2_trackpad_remove(struct hid_device *hdev)
{
	hid_hw_stop(hdev);
}

#define T2_TRACKPAD_DEVICE(model) \
	{ HID_USB_DEVICE(USB_VENDOR_ID_APPLE, \
		USB_DEVICE_ID_APPLE_WELLSPRINGT2_##model) }

static const struct hid_device_id t2_trackpad_devices[] = {
	T2_TRACKPAD_DEVICE(J140K),
	T2_TRACKPAD_DEVICE(J132),
	T2_TRACKPAD_DEVICE(J680),
	T2_TRACKPAD_DEVICE(J680_ALT),
	T2_TRACKPAD_DEVICE(J213),
	T2_TRACKPAD_DEVICE(J214K),
	T2_TRACKPAD_DEVICE(J223),
	T2_TRACKPAD_DEVICE(J230K),
	T2_TRACKPAD_DEVICE(J152F),
	{ }
};
MODULE_DEVICE_TABLE(hid, t2_trackpad_devices);

static struct hid_driver t2_trackpad_driver = {
	.name = "t2_precision_trackpad",
	.id_table = t2_trackpad_devices,
	.probe = t2_trackpad_probe,
	.remove = t2_trackpad_remove,
	.raw_event = t2_trackpad_raw_event,
	.event = t2_trackpad_event,
	.input_mapping = t2_trackpad_input_mapping,
	.input_configured = t2_trackpad_input_configured,
	/*
	 * No .suspend/.resume/.reset_resume here. macOS's own userspace HID
	 * plugin does the same for this exact device class: both
	 * MTTrackpadEmbeddedHIDManager::setPowerState and
	 * ::setPowerStateWithReset are stubs that log "Unsupported" and
	 * return kIOReturnUnsupported. Power management for the internal T2
	 * trackpad happens below this layer, in the transport. There is
	 * nothing for the touch parser itself to do on suspend/resume.
	 */
};
module_hid_driver(t2_trackpad_driver);

MODULE_AUTHOR("André Eikmeyer <andre.eikmeyer@kait2en.org>");
MODULE_DESCRIPTION("Apple T2 internal trackpad HID driver");
MODULE_LICENSE("GPL");
MODULE_VERSION("0.01");
