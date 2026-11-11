/* SPDX-License-Identifier: GPL-2.0 */
/*
 * Apple T2 audio, timestamp decoding
 *
 * Copyright (C) Paul Pawlowski <paul@mrarm.io>
 * Copyright (C) André Eikmeyer <andre.eikmeyer@kait2en.org>
 */

#ifndef T2AUDIO_TIMING_H
#define T2AUDIO_TIMING_H

#ifdef __KERNEL__
#include <linux/types.h>
#else
#include <stdbool.h>
#include <stdint.h>
typedef u64 u64;
#endif

/* Decode the nonnegative, integral IEEE-754 sampleTime without kernel FP. */
static inline bool t2audio_decode_sample_time(u64 bits, u64 *frames)
{
	unsigned int exponent = (bits >> 52) & 0x7ff;
	u64 fraction = bits & ((1ULL << 52) - 1);
	unsigned int shift;

	if (!bits) {
		*frames = 0;
		return true;
	}
	/* Up to 2^53 frames: doubles still represent every integer exactly. */
	if ((bits >> 63) || exponent < 1023 || exponent > 1076 ||
	    (exponent == 1076 && fraction))
		return false;
	fraction |= 1ULL << 52;
	if (exponent == 1076) {
		*frames = fraction << 1;
		return true;
	}
	shift = 1075 - exponent;
	if (fraction & ((1ULL << shift) - 1))
		return false;
	*frames = fraction >> shift;
	return true;
}

/*
 * The position is the last sampleTime as is. Capture additionally holds
 * back the device input latency.
 */
static inline u64 t2audio_position_frames(u64 sample, u64 latency)
{
	return sample > latency ? sample - latency : 0;
}

#endif
