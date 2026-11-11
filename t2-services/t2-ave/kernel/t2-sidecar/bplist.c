#include "bplist.h"

#include <linux/errno.h>
#include <linux/kernel.h>
#include <linux/string.h>
#include <linux/unaligned.h>

#define BP_HEADER	"bplist00"
#define BP_TRAILER_LEN	32

void t2sc_bpw_init(struct t2sc_bpw *w)
{
	w->count = 0;
}

void t2sc_bpw_add(struct t2sc_bpw *w, const char *key, enum t2sc_bpw_type type, s64 value)
{
	if (WARN_ON(w->count >= T2SC_BPW_MAX_ENTRIES))
		return;
	w->entries[w->count++] = (struct t2sc_bpw_entry) {
		.key = key, .type = type, .value = value,
	};
}

struct bp_out {
	u8 *buf;
	size_t size;
	size_t pos;
	u16 offsets[2 * T2SC_BPW_MAX_ENTRIES + 1];
	unsigned int objects;
};

static void bp_put(struct bp_out *o, const void *data, size_t len)
{
	if (o->pos + len <= o->size)
		memcpy(o->buf + o->pos, data, len);
	o->pos += len;
}

static void bp_put_u8(struct bp_out *o, u8 value)
{
	bp_put(o, &value, 1);
}

static void bp_begin_object(struct bp_out *o)
{
	o->offsets[o->objects++] = o->pos;
}

static void bp_put_ascii(struct bp_out *o, const char *str)
{
	size_t len = strlen(str);

	bp_begin_object(o);
	if (len < 15) {
		bp_put_u8(o, 0x50 | len);
	} else {
		bp_put_u8(o, 0x5f);
		bp_put_u8(o, 0x10);
		bp_put_u8(o, len);
	}
	bp_put(o, str, len);
}

static int bp_finish(struct bp_out *o)
{
	size_t table = o->pos;
	__be64 be;
	u8 trailer[BP_TRAILER_LEN] = { 0 };
	unsigned int i;

	for (i = 0; i < o->objects; i++) {
		__be16 off = cpu_to_be16(o->offsets[i]);

		bp_put(o, &off, sizeof(off));
	}
	trailer[6] = 2;		/* offset table entry size */
	trailer[7] = 1;		/* object reference size */
	be = cpu_to_be64(o->objects);
	memcpy(&trailer[8], &be, 8);
	be = cpu_to_be64(0);	/* top object */
	memcpy(&trailer[16], &be, 8);
	be = cpu_to_be64(table);
	memcpy(&trailer[24], &be, 8);
	bp_put(o, trailer, sizeof(trailer));

	if (o->pos > o->size || o->pos > U16_MAX)
		return -ENOSPC;
	return o->pos;
}

int t2sc_bpw_dict(const struct t2sc_bpw *w, u8 *out, size_t size)
{
	struct bp_out o = { .buf = out, .size = size };
	unsigned int n = w->count, i;

	bp_put(&o, BP_HEADER, 8);

	/* Object 0 is the dictionary, keys follow as 1..n and values as n+1..2n. */
	bp_begin_object(&o);
	bp_put_u8(&o, 0xd0 | n);
	for (i = 0; i < n; i++)
		bp_put_u8(&o, 1 + i);
	for (i = 0; i < n; i++)
		bp_put_u8(&o, 1 + n + i);

	for (i = 0; i < n; i++)
		bp_put_ascii(&o, w->entries[i].key);

	for (i = 0; i < n; i++) {
		const struct t2sc_bpw_entry *e = &w->entries[i];
		__be64 be;

		bp_begin_object(&o);
		switch (e->type) {
		case T2SC_BPW_BOOL:
			bp_put_u8(&o, e->value ? 0x09 : 0x08);
			break;
		case T2SC_BPW_INT:
			bp_put_u8(&o, 0x13);
			be = cpu_to_be64(e->value);
			bp_put(&o, &be, 8);
			break;
		case T2SC_BPW_EMPTY_DICT:
			bp_put_u8(&o, 0xd0);
			break;
		}
	}
	return bp_finish(&o);
}

int t2sc_bpw_string(const char *str, u8 *out, size_t size)
{
	struct bp_out o = { .buf = out, .size = size };

	bp_put(&o, BP_HEADER, 8);
	bp_put_ascii(&o, str);
	return bp_finish(&o);
}

static u64 bp_be(const u8 *p, unsigned int size)
{
	u64 value = 0;

	while (size--)
		value = (value << 8) | *p++;
	return value;
}

int t2sc_bpr_open(struct t2sc_bpr *r, const u8 *data, size_t len)
{
	const u8 *t;
	u64 table;

	if (len < 8 + BP_TRAILER_LEN || memcmp(data, BP_HEADER, 8))
		return -EINVAL;
	t = data + len - BP_TRAILER_LEN;
	r->data = data;
	r->len = len;
	r->offset_size = t[6];
	r->ref_size = t[7];
	r->num_objects = get_unaligned_be64(t + 8);
	r->top = get_unaligned_be64(t + 16);
	table = get_unaligned_be64(t + 24);
	if (!r->offset_size || r->offset_size > 8 || !r->ref_size || r->ref_size > 8)
		return -EINVAL;
	if (r->top >= r->num_objects || table < 8 || table >= len - BP_TRAILER_LEN ||
	    r->num_objects > (len - BP_TRAILER_LEN - table) / r->offset_size)
		return -EINVAL;
	r->offsets = data + table;
	return 0;
}

s64 t2sc_bpr_root(const struct t2sc_bpr *r)
{
	return r->top;
}

/* Returns the position of the object's first byte after its marker. */
static int bp_object(const struct t2sc_bpr *r, s64 obj, u8 *marker, u64 *count, size_t *pos)
{
	size_t limit = r->offsets - r->data;
	u64 off;
	u8 m;

	if (obj < 0 || obj >= r->num_objects)
		return -EINVAL;
	off = bp_be(r->offsets + obj * r->offset_size, r->offset_size);
	if (off < 8 || off >= limit)
		return -EINVAL;
	m = r->data[off++];
	*marker = m;
	*count = m & 0x0f;
	if ((m & 0xf0) != 0x00 && (m & 0xf0) != 0x10 && (m & 0x0f) == 0x0f) {
		unsigned int size;

		if (off >= limit || (r->data[off] & 0xf0) != 0x10)
			return -EINVAL;
		size = 1u << (r->data[off] & 0x0f);
		if (size > 8 || off + 1 + size > limit)
			return -EINVAL;
		*count = bp_be(r->data + off + 1, size);
		off += 1 + size;
	}
	*pos = off;
	return 0;
}

static s64 bp_ref(const struct t2sc_bpr *r, size_t pos)
{
	if (pos + r->ref_size > (size_t)(r->offsets - r->data))
		return -EINVAL;
	return bp_be(r->data + pos, r->ref_size);
}

static bool bp_key_equals(const struct t2sc_bpr *r, s64 obj, const char *key)
{
	size_t len = strlen(key), pos;
	u64 count;
	u8 marker;

	if (bp_object(r, obj, &marker, &count, &pos) || (marker & 0xf0) != 0x50)
		return false;
	return count == len && pos + len <= (size_t)(r->offsets - r->data) &&
	       !memcmp(r->data + pos, key, len);
}

s64 t2sc_bpr_dict_get(const struct t2sc_bpr *r, s64 dict, const char *key)
{
	size_t pos;
	u64 count, i;
	u8 marker;
	int ret;

	ret = bp_object(r, dict, &marker, &count, &pos);
	if (ret)
		return ret;
	if ((marker & 0xf0) != 0xd0)
		return -EINVAL;
	for (i = 0; i < count; i++) {
		s64 k = bp_ref(r, pos + i * r->ref_size);

		if (k < 0)
			return k;
		if (bp_key_equals(r, k, key))
			return bp_ref(r, pos + (count + i) * r->ref_size);
	}
	return -ENOENT;
}

s64 t2sc_bpr_array_get(const struct t2sc_bpr *r, s64 array, unsigned int index)
{
	size_t pos;
	u64 count;
	u8 marker;
	int ret;

	ret = bp_object(r, array, &marker, &count, &pos);
	if (ret)
		return ret;
	if ((marker & 0xf0) != 0xa0)
		return -EINVAL;
	if (index >= count)
		return -ENOENT;
	return bp_ref(r, pos + index * r->ref_size);
}

int t2sc_bpr_int(const struct t2sc_bpr *r, s64 obj, s64 *value)
{
	unsigned int size;
	size_t pos;
	u64 count;
	u8 marker;
	int ret;

	ret = bp_object(r, obj, &marker, &count, &pos);
	if (ret)
		return ret;
	if ((marker & 0xf0) != 0x10)
		return -EINVAL;
	size = 1u << (marker & 0x0f);
	if (size > 8 || pos + size > (size_t)(r->offsets - r->data))
		return -EINVAL;
	*value = (s64)bp_be(r->data + pos, size);
	return 0;
}

int t2sc_bpr_bool(const struct t2sc_bpr *r, s64 obj, bool *value)
{
	size_t pos;
	u64 count;
	u8 marker;
	int ret;

	ret = bp_object(r, obj, &marker, &count, &pos);
	if (ret)
		return ret;
	if (marker != 0x08 && marker != 0x09)
		return -EINVAL;
	*value = marker == 0x09;
	return 0;
}

int t2sc_bpr_data(const struct t2sc_bpr *r, s64 obj, const u8 **ptr, size_t *len)
{
	size_t pos;
	u64 count;
	u8 marker;
	int ret;

	ret = bp_object(r, obj, &marker, &count, &pos);
	if (ret)
		return ret;
	if ((marker & 0xf0) != 0x40 || pos + count > (size_t)(r->offsets - r->data))
		return -EINVAL;
	*ptr = r->data + pos;
	*len = count;
	return 0;
}
