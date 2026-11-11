#ifndef T2_SIDECAR_BPLIST_H
#define T2_SIDECAR_BPLIST_H

#include <linux/types.h>

/*
 * Minimal binary property list support for the Sidecar protocol. The writer
 * produces one flat dictionary, the reader walks the few nested dictionaries,
 * arrays and data objects the T2 sends back.
 */

#define T2SC_BPW_MAX_ENTRIES	12

enum t2sc_bpw_type {
	T2SC_BPW_BOOL,
	T2SC_BPW_INT,
	T2SC_BPW_EMPTY_DICT,
};

struct t2sc_bpw_entry {
	const char *key;
	enum t2sc_bpw_type type;
	s64 value;
};

struct t2sc_bpw {
	struct t2sc_bpw_entry entries[T2SC_BPW_MAX_ENTRIES];
	unsigned int count;
};

void t2sc_bpw_init(struct t2sc_bpw *w);
void t2sc_bpw_add(struct t2sc_bpw *w, const char *key, enum t2sc_bpw_type type, s64 value);
/* Returns the encoded length or -ENOSPC. */
int t2sc_bpw_dict(const struct t2sc_bpw *w, u8 *out, size_t size);
int t2sc_bpw_string(const char *str, u8 *out, size_t size);

struct t2sc_bpr {
	const u8 *data;
	size_t len;
	const u8 *offsets;
	u8 offset_size;
	u8 ref_size;
	u64 num_objects;
	u64 top;
};

int t2sc_bpr_open(struct t2sc_bpr *r, const u8 *data, size_t len);
/* Object lookups return an object index or a negative errno. */
s64 t2sc_bpr_root(const struct t2sc_bpr *r);
s64 t2sc_bpr_dict_get(const struct t2sc_bpr *r, s64 dict, const char *key);
s64 t2sc_bpr_array_get(const struct t2sc_bpr *r, s64 array, unsigned int index);
int t2sc_bpr_int(const struct t2sc_bpr *r, s64 obj, s64 *value);
int t2sc_bpr_bool(const struct t2sc_bpr *r, s64 obj, bool *value);
int t2sc_bpr_data(const struct t2sc_bpr *r, s64 obj, const u8 **ptr, size_t *len);

#endif
