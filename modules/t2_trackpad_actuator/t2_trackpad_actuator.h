/* SPDX-License-Identifier: GPL-2.0-only */
/* Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org> */
#ifndef T2_TRACKPAD_ACTUATOR_H
#define T2_TRACKPAD_ACTUATOR_H

#include <linux/types.h>

/*
 * Fire a Taptic Engine waveform. See t2_trackpad_actuator.c for the
 * waveform_id/strength meaning.
 */
int t2_actuator_fire(u8 waveform_id, u8 strength);

#endif
