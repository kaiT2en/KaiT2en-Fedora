/* SPDX-License-Identifier: GPL-2.0 */
/*
 * Apple T2 virtual USB host controller
 *
 * Copyright (C) Paul Pawlowski <paul@mrarm.io>
 * Copyright (C) André Eikmeyer <andre.eikmeyer@kait2en.org>
 */

#ifndef BCE_VHCI_H
#define BCE_VHCI_H

#include "apple-t2-vhci-queue.h"
#include "apple-t2-vhci-transfer.h"

struct device;
struct usb_hcd;
struct apple_bce;

struct bce_vhci_device {
	struct bce_vhci_transfer_queue tq[32];
	u32 tq_mask;
};

struct bce_vhci {
	struct apple_bce *bce;
	struct device *vdev;
	struct usb_hcd *hcd;
	struct bce_vhci_message_queue msg_commands;
	struct bce_vhci_message_queue msg_system;
	struct bce_vhci_message_queue msg_isochronous;
	struct bce_vhci_message_queue msg_interrupt;
	struct bce_vhci_message_queue msg_asynchronous;
	spinlock_t msg_asynchronous_lock; /* keeps request and buffer in order */
	struct bce_vhci_command_queue cq;
	struct apple_bce_cq *ev_cq;
	struct bce_vhci_event_queue ev_commands;
	struct bce_vhci_event_queue ev_system;
	struct bce_vhci_event_queue ev_isochronous;
	struct bce_vhci_event_queue ev_interrupt;
	struct bce_vhci_event_queue ev_asynchronous;
	u16 port_mask;
	u8 port_count;
	u16 port_power_mask;
	bce_vhci_device_t port_to_device[17];
	struct bce_vhci_device *devices[17];
	struct workqueue_struct *tq_state_wq;
	struct work_struct w_fw_events;
	unsigned long port_change_pending;
	/* Ports whose device must be reset-resumed after a stateful resume. */
	unsigned long stateful_reset_ports;
	bool hcd_registered;
	bool system_suspending;
};

int bce_vhci_start(struct usb_hcd *hcd);
void bce_vhci_stop(struct usb_hcd *hcd);

struct bce_vhci *bce_vhci_from_hcd(struct usb_hcd *hcd);

#endif //BCE_VHCI_H
