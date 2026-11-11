// SPDX-License-Identifier: GPL-2.0
/*
 * Apple T2 audio, message encoding
 *
 * Copyright (C) Paul Pawlowski <paul@mrarm.io>
 * Copyright (C) André Eikmeyer <andre.eikmeyer@kait2en.org>
 */

#define pr_fmt(fmt) KBUILD_MODNAME ": " fmt

#include "protocol.h"
#include "protocol_bce.h"
#include "audio.h"
#include <linux/unaligned.h>

int t2audio_msg_read_base(struct t2audio_msg *msg,
			  struct t2audio_msg_base *base)
{
	if (msg->size < sizeof(struct t2audio_msg_header) +
				sizeof(struct t2audio_msg_base) * 2)
		return -EINVAL;
	*base = *((struct t2audio_msg_base *)((struct t2audio_msg_header *)
						      msg->data +
					      1));
	return 0;
}

/*
 * Reads fields of a message from the T2 in order. A read past the end of the
 * message returns 0 and marks the reader as failed, the caller checks
 * t2audio_read_ok() once at the end.
 */
struct t2audio_reader {
	struct t2audio_msg *msg;
	size_t offset;
	bool overrun;
};

static bool t2audio_read_avail(struct t2audio_reader *r, size_t size)
{
	if (r->overrun || size > r->msg->size ||
	    r->offset > r->msg->size - size) {
		r->overrun = true;
		return false;
	}
	return true;
}

static int t2audio_read_ok(struct t2audio_reader *r)
{
	return r->overrun ? -EINVAL : 0;
}

/* Checks the message type and positions the reader behind the base. */
static int t2audio_read_start(struct t2audio_reader *r, struct t2audio_msg *msg,
			      u32 type)
{
	r->msg = msg;
	r->offset = sizeof(struct t2audio_msg_header) +
		    sizeof(struct t2audio_msg_base);
	r->overrun = false;
	if (msg->size < r->offset)
		return -EINVAL;
	if (((struct t2audio_msg_base *)((struct t2audio_msg_header *)msg->data +
					 1))
		    ->msg != type)
		return -EINVAL;
	return 0;
}

static t2audio_device_id_t t2audio_read_devid(struct t2audio_reader *r)
{
	return ((struct t2audio_msg_header *)r->msg->data)->device_id;
}

static u32 t2audio_read_u32(struct t2audio_reader *r)
{
	if (!t2audio_read_avail(r, sizeof(u32)))
		return 0;
	r->offset += sizeof(u32);
	return *(u32 *)((u8 *)r->msg->data + r->offset - sizeof(u32));
}

static u64 t2audio_read_u64(struct t2audio_reader *r)
{
	if (!t2audio_read_avail(r, sizeof(u64)))
		return 0;
	r->offset += sizeof(u64);
	return *(u64 *)((u8 *)r->msg->data + r->offset - sizeof(u64));
}

/* Returns the next size bytes, NULL if the message is shorter. */
static void *t2audio_read_ptr(struct t2audio_reader *r, size_t size)
{
	if (!t2audio_read_avail(r, size))
		return NULL;
	return (u8 *)r->msg->data + r->offset;
}

static void t2audio_read_prop(struct t2audio_reader *r,
			      struct t2audio_prop_addr *prop)
{
	prop->element = t2audio_read_u32(r);
	prop->scope = t2audio_read_u32(r);
	prop->selector = t2audio_read_u32(r);
}

int t2audio_msg_read_start_io_response(struct t2audio_msg *msg)
{
	struct t2audio_reader r;

	return t2audio_read_start(&r, msg, T2AUDIO_MSG_START_IO_RESPONSE);
}

int t2audio_msg_read_stop_io_response(struct t2audio_msg *msg)
{
	struct t2audio_reader r;

	return t2audio_read_start(&r, msg, T2AUDIO_MSG_STOP_IO_RESPONSE);
}

int t2audio_msg_read_update_timestamp(struct t2audio_msg *msg,
				      t2audio_device_id_t *devid,
				      u64 *timestamp, u64 *update_seed,
				      u64 *sample_time)
{
	size_t offset = sizeof(struct t2audio_msg_header) +
			sizeof(struct t2audio_msg_base);
	const u8 *data = msg->data;

	/* payload: timestamp, update seed, sampleTime as IEEE 754 double */
	if (msg->size < offset + 3 * sizeof(u64) ||
	    get_unaligned_le32(data + sizeof(struct t2audio_msg_header)) !=
		    T2AUDIO_MSG_UPDATE_TIMESTAMP)
		return -EINVAL;
	*devid = get_unaligned_le64(
		data + offsetof(struct t2audio_msg_header, device_id));
	*timestamp = get_unaligned_le64(data + offset);
	*update_seed = get_unaligned_le64(data + offset + 8);
	*sample_time = get_unaligned_le64(data + offset + 16);
	return 0;
}

int t2audio_msg_read_get_property_response(struct t2audio_msg *msg,
					   t2audio_object_id_t *obj,
					   struct t2audio_prop_addr *prop,
					   void **data, u64 *data_size)
{
	struct t2audio_reader r;
	int ret;

	ret = t2audio_read_start(&r, msg, T2AUDIO_MSG_GET_PROPERTY_RESPONSE);
	if (ret)
		return ret;
	*obj = t2audio_read_u64(&r);
	t2audio_read_prop(&r, prop);
	*data_size = t2audio_read_u64(&r);
	*data = t2audio_read_ptr(&r, *data_size);
	return t2audio_read_ok(&r);
}

int t2audio_msg_read_set_property_response(struct t2audio_msg *msg,
					   t2audio_object_id_t *obj)
{
	struct t2audio_reader r;
	int ret;

	ret = t2audio_read_start(&r, msg, T2AUDIO_MSG_SET_PROPERTY_RESPONSE);
	if (ret)
		return ret;
	*obj = t2audio_read_u64(&r);
	return t2audio_read_ok(&r);
}

int t2audio_msg_read_property_listener_response(struct t2audio_msg *msg,
						t2audio_object_id_t *obj,
						struct t2audio_prop_addr *prop)
{
	struct t2audio_reader r;
	int ret;

	ret = t2audio_read_start(&r, msg,
				 T2AUDIO_MSG_PROPERTY_LISTENER_RESPONSE);
	if (ret)
		return ret;
	*obj = t2audio_read_u64(&r);
	t2audio_read_prop(&r, prop);
	return t2audio_read_ok(&r);
}

int t2audio_msg_read_property_changed(struct t2audio_msg *msg,
				      t2audio_device_id_t *devid,
				      t2audio_object_id_t *obj,
				      struct t2audio_prop_addr *prop)
{
	struct t2audio_reader r;
	int ret;

	ret = t2audio_read_start(&r, msg, T2AUDIO_MSG_PROPERTY_CHANGED);
	if (ret)
		return ret;
	*devid = t2audio_read_devid(&r);
	*obj = t2audio_read_u64(&r);
	t2audio_read_prop(&r, prop);
	return t2audio_read_ok(&r);
}

int t2audio_msg_read_set_input_stream_address_ranges_response(
	struct t2audio_msg *msg)
{
	struct t2audio_reader r;

	return t2audio_read_start(
		&r, msg, T2AUDIO_MSG_SET_INPUT_STREAM_ADDRESS_RANGES_RESPONSE);
}

static int t2audio_read_id_list(struct t2audio_msg *msg, u32 type,
				t2audio_object_id_t **list, u64 *count)
{
	struct t2audio_reader r;
	int ret;

	ret = t2audio_read_start(&r, msg, type);
	if (ret)
		return ret;
	*count = t2audio_read_u64(&r);
	if (*count > SIZE_MAX / sizeof(**list))
		return -EINVAL;
	*list = t2audio_read_ptr(&r, *count * sizeof(**list));
	return t2audio_read_ok(&r);
}

int t2audio_msg_read_get_input_stream_list_response(struct t2audio_msg *msg,
						    t2audio_object_id_t **str_l,
						    u64 *str_cnt)
{
	return t2audio_read_id_list(msg,
				    T2AUDIO_MSG_GET_INPUT_STREAM_LIST_RESPONSE,
				    str_l, str_cnt);
}

int t2audio_msg_read_get_output_stream_list_response(
	struct t2audio_msg *msg, t2audio_object_id_t **str_l, u64 *str_cnt)
{
	return t2audio_read_id_list(msg,
				    T2AUDIO_MSG_GET_OUTPUT_STREAM_LIST_RESPONSE,
				    str_l, str_cnt);
}

int t2audio_msg_read_set_remote_access_response(struct t2audio_msg *msg)
{
	struct t2audio_reader r;

	return t2audio_read_start(&r, msg,
				  T2AUDIO_MSG_SET_REMOTE_ACCESS_RESPONSE);
}

int t2audio_msg_read_get_device_list_response(struct t2audio_msg *msg,
					      t2audio_device_id_t **dev_l,
					      u64 *dev_cnt)
{
	return t2audio_read_id_list(msg, T2AUDIO_MSG_GET_DEVICE_LIST_RESPONSE,
				    dev_l, dev_cnt);
}

struct t2audio_writer {
	struct t2audio_msg *msg;
	size_t offset;
};

static void t2audio_write_u32(struct t2audio_writer *w, u32 value)
{
	*(u32 *)((u8 *)w->msg->data + w->offset) = value;
	w->offset += sizeof(value);
}

static void t2audio_write_u64(struct t2audio_writer *w, u64 value)
{
	*(u64 *)((u8 *)w->msg->data + w->offset) = value;
	w->offset += sizeof(value);
}

static void t2audio_write_bin(struct t2audio_writer *w, const void *value,
			      size_t size)
{
	memcpy((u8 *)w->msg->data + w->offset, value, size);
	w->offset += size;
}

static void t2audio_write_prop(struct t2audio_writer *w,
			       struct t2audio_prop_addr prop)
{
	t2audio_write_u32(w, prop.element);
	t2audio_write_u32(w, prop.scope);
	t2audio_write_u32(w, prop.selector);
}

/* Writes the header and the base. The tag is set when the message is sent. */
static void t2audio_write_start(struct t2audio_writer *w,
				struct t2audio_msg *msg, u8 type,
				t2audio_device_id_t devid, u32 msg_type)
{
	struct t2audio_msg_header *header = msg->data;

	w->msg = msg;
	w->offset = sizeof(*header);
	header->type = type;
	header->device_id = devid;
	t2audio_write_u32(w, msg_type);
	t2audio_write_u32(w, 0);
}

static void t2audio_write_end(struct t2audio_writer *w)
{
	w->msg->size = w->offset;
}

static void t2audio_write_command(struct t2audio_msg *msg,
				  t2audio_device_id_t devid, u32 msg_type)
{
	struct t2audio_writer w;

	t2audio_write_start(&w, msg, T2AUDIO_MSG_TYPE_COMMAND, devid, msg_type);
	t2audio_write_end(&w);
}

void t2audio_msg_write_start_io(struct t2audio_msg *msg,
				t2audio_device_id_t dev)
{
	t2audio_write_command(msg, dev, T2AUDIO_MSG_START_IO);
}

void t2audio_msg_write_stop_io(struct t2audio_msg *msg, t2audio_device_id_t dev)
{
	t2audio_write_command(msg, dev, T2AUDIO_MSG_STOP_IO);
}

void t2audio_msg_write_get_property(struct t2audio_msg *msg,
				    t2audio_device_id_t dev,
				    t2audio_object_id_t obj,
				    struct t2audio_prop_addr prop,
				    void *qualifier, u64 qualifier_size)
{
	struct t2audio_writer w;

	t2audio_write_start(&w, msg, T2AUDIO_MSG_TYPE_COMMAND, dev,
			    T2AUDIO_MSG_GET_PROPERTY);
	t2audio_write_u64(&w, obj);
	t2audio_write_prop(&w, prop);
	t2audio_write_u64(&w, qualifier_size);
	t2audio_write_bin(&w, qualifier, qualifier_size);
	t2audio_write_end(&w);
}

void t2audio_msg_write_set_property(struct t2audio_msg *msg,
				    t2audio_device_id_t dev,
				    t2audio_object_id_t obj,
				    struct t2audio_prop_addr prop, void *data,
				    u64 data_size, void *qualifier,
				    u64 qualifier_size)
{
	struct t2audio_writer w;

	t2audio_write_start(&w, msg, T2AUDIO_MSG_TYPE_COMMAND, dev,
			    T2AUDIO_MSG_SET_PROPERTY);
	t2audio_write_u64(&w, obj);
	t2audio_write_prop(&w, prop);
	t2audio_write_u64(&w, data_size);
	t2audio_write_bin(&w, data, data_size);
	t2audio_write_u64(&w, qualifier_size);
	t2audio_write_bin(&w, qualifier, qualifier_size);
	t2audio_write_end(&w);
}

void t2audio_msg_write_property_listener(struct t2audio_msg *msg,
					 t2audio_device_id_t dev,
					 t2audio_object_id_t obj,
					 struct t2audio_prop_addr prop)
{
	struct t2audio_writer w;

	t2audio_write_start(&w, msg, T2AUDIO_MSG_TYPE_COMMAND, dev,
			    T2AUDIO_MSG_PROPERTY_LISTENER);
	t2audio_write_u64(&w, obj);
	t2audio_write_prop(&w, prop);
	t2audio_write_end(&w);
}

void t2audio_msg_write_set_input_stream_address_ranges(
	struct t2audio_msg *msg, t2audio_device_id_t devid)
{
	t2audio_write_command(msg, devid,
			      T2AUDIO_MSG_SET_INPUT_STREAM_ADDRESS_RANGES);
}

void t2audio_msg_write_get_input_stream_list(struct t2audio_msg *msg,
					     t2audio_device_id_t devid)
{
	t2audio_write_command(msg, devid, T2AUDIO_MSG_GET_INPUT_STREAM_LIST);
}

void t2audio_msg_write_get_output_stream_list(struct t2audio_msg *msg,
					      t2audio_device_id_t devid)
{
	t2audio_write_command(msg, devid, T2AUDIO_MSG_GET_OUTPUT_STREAM_LIST);
}

void t2audio_msg_write_set_remote_access(struct t2audio_msg *msg, u64 mode)
{
	struct t2audio_writer w;

	t2audio_write_start(&w, msg, T2AUDIO_MSG_TYPE_COMMAND, 0,
			    T2AUDIO_MSG_SET_REMOTE_ACCESS);
	t2audio_write_u64(&w, mode);
	t2audio_write_end(&w);
}

void t2audio_msg_write_alive_notification(struct t2audio_msg *msg,
					  u32 proto_ver, u32 msg_ver)
{
	struct t2audio_writer w;

	t2audio_write_start(&w, msg, T2AUDIO_MSG_TYPE_NOTIFICATION, 0,
			    T2AUDIO_MSG_NOTIFICATION_ALIVE);
	t2audio_write_u32(&w, proto_ver);
	t2audio_write_u32(&w, msg_ver);
	t2audio_write_end(&w);
}

void t2audio_msg_write_update_timestamp_response(struct t2audio_msg *msg)
{
	struct t2audio_writer w;

	t2audio_write_start(&w, msg, T2AUDIO_MSG_TYPE_RESPONSE, 0,
			    T2AUDIO_MSG_UPDATE_TIMESTAMP_RESPONSE);
	t2audio_write_end(&w);
}

void t2audio_msg_write_get_device_list(struct t2audio_msg *msg)
{
	t2audio_write_command(msg, 0, T2AUDIO_MSG_GET_DEVICE_LIST);
}

int t2audio_cmd_start_io(struct t2audio_device *a, t2audio_device_id_t devid)
{
	struct t2audio_msg reply = t2audio_reply_alloc();
	struct t2audio_send_ctx sctx;
	int status;

	status = t2audio_send_cmd_sync(a, &sctx, &reply, 500,
				       t2audio_msg_write_start_io, devid);
	if (status) {
		t2audio_reply_free(&reply);
		return status;
	}
	status = t2audio_msg_read_start_io_response(&reply);
	t2audio_reply_free(&reply);
	return status;
}

int t2audio_cmd_stop_io(struct t2audio_device *a, t2audio_device_id_t devid)
{
	struct t2audio_msg reply = t2audio_reply_alloc();
	struct t2audio_send_ctx sctx;
	int status;

	status = t2audio_send_cmd_sync(a, &sctx, &reply, 500,
				       t2audio_msg_write_stop_io, devid);
	if (status) {
		t2audio_reply_free(&reply);
		return status;
	}
	status = t2audio_msg_read_stop_io_response(&reply);
	t2audio_reply_free(&reply);
	return status;
}

int t2audio_cmd_get_property(struct t2audio_device *a, struct t2audio_msg *buf,
			     t2audio_device_id_t devid, t2audio_object_id_t obj,
			     struct t2audio_prop_addr prop, void *qualifier,
			     u64 qualifier_size, void **data, u64 *data_size)
{
	struct t2audio_send_ctx sctx;
	int status;

	status = t2audio_send_cmd_sync(a, &sctx, buf, 500,
				       t2audio_msg_write_get_property, devid,
				       obj, prop, qualifier, qualifier_size);
	if (status)
		return status;
	return t2audio_msg_read_get_property_response(buf, &obj, &prop, data,
						      data_size);
}

int t2audio_cmd_get_primitive_property(struct t2audio_device *a,
				       t2audio_device_id_t devid,
				       t2audio_object_id_t obj,
				       struct t2audio_prop_addr prop,
				       void *qualifier, u64 qualifier_size,
				       void *data, u64 data_size)
{
	int status;
	struct t2audio_msg reply = t2audio_reply_alloc();
	void *r_data;
	u64 r_data_size;

	status = t2audio_cmd_get_property(a, &reply, devid, obj, prop,
					  qualifier, qualifier_size, &r_data,
					  &r_data_size);
	if (status)
		goto finish;
	if (r_data_size != data_size) {
		status = -EINVAL;
		goto finish;
	}
	memcpy(data, r_data, data_size);
finish:
	t2audio_reply_free(&reply);
	return status;
}

int t2audio_cmd_set_property(struct t2audio_device *a,
			     t2audio_device_id_t devid, t2audio_object_id_t obj,
			     struct t2audio_prop_addr prop, void *qualifier,
			     u64 qualifier_size, void *data, u64 data_size)
{
	struct t2audio_msg reply = t2audio_reply_alloc();
	struct t2audio_send_ctx sctx;
	int status;

	status = t2audio_send_cmd_sync(a, &sctx, &reply, 500,
				       t2audio_msg_write_set_property, devid,
				       obj, prop, data, data_size, qualifier,
				       qualifier_size);
	if (status) {
		t2audio_reply_free(&reply);
		return status;
	}
	status = t2audio_msg_read_set_property_response(&reply, &obj);
	t2audio_reply_free(&reply);
	return status;
}

int t2audio_cmd_property_listener(struct t2audio_device *a,
				  t2audio_device_id_t devid,
				  t2audio_object_id_t obj,
				  struct t2audio_prop_addr prop)
{
	struct t2audio_msg reply = t2audio_reply_alloc();
	struct t2audio_send_ctx sctx;
	int status;

	status = t2audio_send_cmd_sync(a, &sctx, &reply, 500,
				       t2audio_msg_write_property_listener,
				       devid, obj, prop);
	if (status) {
		t2audio_reply_free(&reply);
		return status;
	}
	status = t2audio_msg_read_property_listener_response(&reply, &obj,
							     &prop);
	t2audio_reply_free(&reply);
	return status;
}

int t2audio_cmd_set_input_stream_address_ranges(struct t2audio_device *a,
						t2audio_device_id_t devid)
{
	struct t2audio_msg reply = t2audio_reply_alloc();
	struct t2audio_send_ctx sctx;
	int status;

	status = t2audio_send_cmd_sync(
		a, &sctx, &reply, 500,
		t2audio_msg_write_set_input_stream_address_ranges, devid);
	if (status) {
		t2audio_reply_free(&reply);
		return status;
	}
	status = t2audio_msg_read_set_input_stream_address_ranges_response(
		&reply);
	t2audio_reply_free(&reply);
	return status;
}

int t2audio_cmd_get_input_stream_list(struct t2audio_device *a,
				      struct t2audio_msg *buf,
				      t2audio_device_id_t devid,
				      t2audio_object_id_t **str_l, u64 *str_cnt)
{
	struct t2audio_send_ctx sctx;
	int status;

	status = t2audio_send_cmd_sync(a, &sctx, buf, 500,
				       t2audio_msg_write_get_input_stream_list,
				       devid);
	if (status)
		return status;
	return t2audio_msg_read_get_input_stream_list_response(buf, str_l,
							       str_cnt);
}

int t2audio_cmd_get_output_stream_list(struct t2audio_device *a,
				       struct t2audio_msg *buf,
				       t2audio_device_id_t devid,
				       t2audio_object_id_t **str_l,
				       u64 *str_cnt)
{
	struct t2audio_send_ctx sctx;
	int status;

	status = t2audio_send_cmd_sync(a, &sctx, buf, 500,
				       t2audio_msg_write_get_output_stream_list,
				       devid);
	if (status)
		return status;
	return t2audio_msg_read_get_output_stream_list_response(buf, str_l,
								str_cnt);
}

int t2audio_cmd_set_remote_access(struct t2audio_device *a, u64 mode)
{
	struct t2audio_msg reply = t2audio_reply_alloc();
	struct t2audio_send_ctx sctx;
	int status;

	status = t2audio_send_cmd_sync(a, &sctx, &reply, 500,
				       t2audio_msg_write_set_remote_access,
				       mode);
	if (status) {
		t2audio_reply_free(&reply);
		return status;
	}
	status = t2audio_msg_read_set_remote_access_response(&reply);
	t2audio_reply_free(&reply);
	return status;
}

int t2audio_cmd_get_device_list(struct t2audio_device *a,
				struct t2audio_msg *buf,
				t2audio_device_id_t **dev_l, u64 *dev_cnt)
{
	struct t2audio_send_ctx sctx;
	int status;

	status = t2audio_send_cmd_sync(a, &sctx, buf, 500,
				       t2audio_msg_write_get_device_list);
	if (status)
		return status;
	return t2audio_msg_read_get_device_list_response(buf, dev_l, dev_cnt);
}
