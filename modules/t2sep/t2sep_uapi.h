/* SPDX-License-Identifier: GPL-2.0-only WITH Linux-syscall-note */
#ifndef T2SEP_UAPI_H
#define T2SEP_UAPI_H

#include <linux/ioctl.h>
#include <linux/types.h>

#define T2SEP_IOC_MAGIC 0xa7

/*
 * One request/response exchange over a SEP mailbox endpoint. The request and
 * response bytes are opaque to the driver: it copies request into the OOL
 * input buffer, posts the descriptor, and copies the OOL output buffer back.
 * The wire format, including any header and integrity digest, is built and
 * checked in user space.
 */
struct t2sep_exchange {
	__u8 endpoint;
	__u8 operation;
	__u8 reserved[2];
	__u32 request_length;
	__u32 response_capacity;
	__u32 response_length;
	__u64 request;
	__u64 response;
};

#define T2SEP_IOC_EXCHANGE _IOWR(T2SEP_IOC_MAGIC, 0, struct t2sep_exchange)

#endif /* T2SEP_UAPI_H */
