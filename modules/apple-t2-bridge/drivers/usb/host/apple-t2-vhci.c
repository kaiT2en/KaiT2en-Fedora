// SPDX-License-Identifier: GPL-2.0
/*
 * Apple T2 virtual USB host controller
 *
 * Copyright (C) Paul Pawlowski <paul@mrarm.io>
 * Copyright (C) André Eikmeyer <andre.eikmeyer@kait2en.org>
 */

#define pr_fmt(fmt) KBUILD_MODNAME ": " fmt

#include "apple-t2-vhci.h"
#include "apple-t2-vhci-command.h"
#include <linux/auxiliary_bus.h>
#include <linux/usb.h>
#include <linux/usb/hcd.h>
#include <linux/module.h>
#include <linux/err.h>
#include <linux/slab.h>

/*
 * bridgeOS exposes several T2-internal devices through a virtual USB 2.0 hub.
 * This HCD translates usbcore hub, device and endpoint operations into the
 * firmware command/event protocol carried over BCE queues.
 */

static const struct hc_driver bce_vhci_driver;
static u16 bce_vhci_port_mask = U16_MAX;

enum bce_vhci_port_status_bits {
	BCE_VHCI_PORT_STATUS_OVERCURRENT = 0x2,
	BCE_VHCI_PORT_STATUS_CONNECTED = 0x4,
	BCE_VHCI_PORT_STATUS_RESET = 0x8,
	BCE_VHCI_PORT_STATUS_ENABLED = 0x10,
	BCE_VHCI_PORT_STATUS_SUSPENDED_LEGACY = 0x60,
	BCE_VHCI_PORT_STATUS_SUSPENDED = 0x80,
	BCE_VHCI_PORT_STATUS_HIGH_SPEED = 0x200,
	BCE_VHCI_PORT_STATUS_C_CONNECTION = 0x40000,
};

static int bce_vhci_create_event_queues(struct bce_vhci *vhci);
static void bce_vhci_destroy_event_queues(struct bce_vhci *vhci);
static int bce_vhci_create_message_queues(struct bce_vhci *vhci);
static void bce_vhci_destroy_message_queues(struct bce_vhci *vhci);
static void bce_vhci_handle_firmware_events_w(struct work_struct *ws);
static void bce_vhci_firmware_event_completion(struct apple_bce_sq *sq);
static int bce_vhci_start_controller(struct bce_vhci *vhci);
static int __bce_vhci_add_hcd(struct bce_vhci *vhci);
static void bce_vhci_remove_hcd(struct bce_vhci *vhci);
static int bce_vhci_create(struct device *dev, struct bce_vhci *vhci)
{
	int status;


	vhci->vdev = dev;

	vhci->tq_state_wq =
		alloc_ordered_workqueue("apple-t2-vhci-tq-state", 0);
	if (!vhci->tq_state_wq) {
		status = -ENOMEM;
		goto fail_wq;
	}
	INIT_WORK(&vhci->w_fw_events, bce_vhci_handle_firmware_events_w);
	vhci->port_change_pending = 0;
	vhci->system_suspending = false;

	status = bce_vhci_create_message_queues(vhci);
	if (status)
		goto fail_mq;
	status = bce_vhci_create_event_queues(vhci);
	if (status)
		goto fail_eq;

	vhci->hcd = usb_create_hcd(&bce_vhci_driver, vhci->vdev,
				   dev_name(vhci->vdev));
	if (!vhci->hcd) {
		status = -ENOMEM;
		goto fail_hcd;
	}
	vhci->hcd->self.sysdev = apple_bce_dma_device(vhci->bce);
	/*
	 * The HCD accepts arbitrary Linux scatterlists and splits them into the
	 * 32-element descriptor blocks supported by bridgeOS.  Segment boundaries
	 * do not have to match USB max-packet boundaries.
	 */
	vhci->hcd->self.sg_tablesize = ~0U;
	vhci->hcd->self.no_sg_constraint = 1;
	*((struct bce_vhci **)vhci->hcd->hcd_priv) = vhci;
	vhci->hcd->speed = HCD_USB2;

	status = __bce_vhci_add_hcd(vhci);
	if (status)
		goto fail_hcd;

	pr_info("device initialized\n");
	return 0;

fail_hcd:
	bce_vhci_destroy_event_queues(vhci);
fail_eq:
	bce_vhci_destroy_message_queues(vhci);
fail_mq:
	destroy_workqueue(vhci->tq_state_wq);
	vhci->tq_state_wq = NULL;
fail_wq:
	if (!status)
		status = -EINVAL;
	return status;
}

static void bce_vhci_destroy(struct bce_vhci *vhci)
{
	bce_vhci_remove_hcd(vhci);
	bce_vhci_destroy_event_queues(vhci);
	bce_vhci_destroy_message_queues(vhci);
	destroy_workqueue(vhci->tq_state_wq);
	vhci->tq_state_wq = NULL;
}

static int __bce_vhci_add_hcd(struct bce_vhci *vhci)
{
	int status;

	status = usb_add_hcd(vhci->hcd, 0, 0);
	if (!status)
		vhci->hcd_registered = true;
	return status;
}

static void bce_vhci_remove_hcd(struct bce_vhci *vhci)
{
	if (!vhci->hcd_registered)
		return;
	usb_remove_hcd(vhci->hcd);
	vhci->hcd_registered = false;
}

static void bce_vhci_shutdown(struct bce_vhci *vhci)
{
	if (vhci->hcd_registered) {
		bce_vhci_remove_hcd(vhci);
		return;
	}

	/*
	 * Shutdown is not remove: the USB core may already have detached the HCD,
	 * but the T2-side controller can still be left enabled. Send the controller
	 * disable directly as a final lights-out command while queues still exist.
	 */
	bce_vhci_cmd_controller_disable(&vhci->cq);
}

static void bce_vhci_resume_event_queues(struct bce_vhci *vhci)
{
	/* System events carry command completions, so restore them first. */
	bce_vhci_event_queue_resume(&vhci->ev_system);
	bce_vhci_event_queue_resume(&vhci->ev_commands);
	bce_vhci_event_queue_resume(&vhci->ev_isochronous);
	bce_vhci_event_queue_resume(&vhci->ev_interrupt);
	bce_vhci_event_queue_resume(&vhci->ev_asynchronous);
}

static int bce_vhci_pause_event_queues(struct bce_vhci *vhci)
{
	struct bce_vhci_event_queue *queues[] = {
		&vhci->ev_isochronous,	&vhci->ev_interrupt,
		&vhci->ev_asynchronous, &vhci->ev_commands,
		&vhci->ev_system,
	};
	int i;
	int status;

	for (i = 0; i < ARRAY_SIZE(queues); i++) {
		status = bce_vhci_event_queue_pause(queues[i]);
		if (status)
			goto rollback;
		if (queues[i] == &vhci->ev_commands)
			flush_workqueue(vhci->tq_state_wq);
	}
	return 0;

rollback:
	while (i >= 0) {
		bce_vhci_event_queue_resume(queues[i]);
		i--;
	}
	return status;
}

static int bce_vhci_pm_prepare(struct bce_vhci *vhci)
{
	int status;

	vhci->port_change_pending = 0;
	flush_workqueue(vhci->tq_state_wq);
	status = bce_vhci_pause_event_queues(vhci);
	if (status)
		WRITE_ONCE(vhci->system_suspending, false);
	return status;
}

struct bce_vhci *bce_vhci_from_hcd(struct usb_hcd *hcd)
{
	return *((struct bce_vhci **)hcd->hcd_priv);
}

int bce_vhci_start(struct usb_hcd *hcd)
{
	struct bce_vhci *vhci = bce_vhci_from_hcd(hcd);

	return bce_vhci_start_controller(vhci);
}

void bce_vhci_stop(struct usb_hcd *hcd)
{
	struct bce_vhci *vhci = bce_vhci_from_hcd(hcd);

	bce_vhci_cmd_controller_disable(&vhci->cq);
}

static int bce_vhci_start_controller(struct bce_vhci *vhci)
{
	int status;
	u16 port_mask = 0;
	bce_vhci_port_t port_no = 0;

	status = bce_vhci_cmd_controller_enable(&vhci->cq, 1, &port_mask);
	if (status)
		return status;
	vhci->port_mask = port_mask;
	vhci->port_power_mask = 0;
	status = bce_vhci_cmd_controller_start(&vhci->cq);
	if (status)
		return status;
	while (port_mask) {
		port_no += 1;
		port_mask >>= 1;
	}
	vhci->port_count = port_no;
	return 0;
}

static int bce_vhci_hub_status_data(struct usb_hcd *hcd, char *buf)
{
	struct bce_vhci *vhci = bce_vhci_from_hcd(hcd);
	unsigned long pending;

	/* Only ports reported by firmware as changed should wake the hub state machine. */
	pending = READ_ONCE(vhci->port_change_pending);
	if (!pending)
		return 0;

	memset(buf, 0, 2);
	buf[0] = (char)((pending << 1) & 0xff);
	buf[1] = (char)((pending >> 7) & 0xff);
	/* Bitmap length in bytes, ports 8 and up live in buf[1] */
	if (!buf[0] && !buf[1])
		return 0;
	return vhci->port_count >= 8 ? 2 : 1;
}

static int bce_vhci_reset_device(struct bce_vhci *vhci, int index, u16 timeout);

static int bce_vhci_hub_control(struct usb_hcd *hcd, u16 type_req, u16 wValue,
				u16 wIndex, char *buf, u16 wLength)
{
	struct bce_vhci *vhci = bce_vhci_from_hcd(hcd);
	int status;
	int port;
	struct usb_hub_descriptor *hd;
	struct usb_hub_status *hs;
	struct usb_port_status *ps;
	u32 port_status;
	bool reset_resume;

	if (type_req == GetHubDescriptor &&
	    wLength >= sizeof(struct usb_hub_descriptor)) {
		hd = (struct usb_hub_descriptor *)buf;
		memset(hd, 0, sizeof(*hd));
		hd->bDescLength = sizeof(struct usb_hub_descriptor);
		hd->bDescriptorType = USB_DT_HUB;
		hd->bNbrPorts = (u8)vhci->port_count;
		hd->wHubCharacteristics = cpu_to_le16(HUB_CHAR_INDV_PORT_LPSM |
						      HUB_CHAR_INDV_PORT_OCPM |
						      HUB_CHAR_COMPOUND);
		hd->bPwrOn2PwrGood = 0;
		hd->bHubContrCurrent = 0;

		/* All devices exposed by the T2 virtual hub are built into the system. */
		for (port = 1; port <= vhci->port_count; port++)
			hd->u.hs.DeviceRemovable[port / 8] |= BIT(port % 8);

		return 0;
	} else if (type_req == GetHubStatus &&
		   wLength >= sizeof(struct usb_hub_status)) {
		hs = (struct usb_hub_status *)buf;
		memset(hs, 0, sizeof(*hs));
		hs->wHubStatus = 0;
		hs->wHubChange = 0;
		return 0;
	} else if (type_req == GetPortStatus && wLength >= 4 /* usb 2.0 */) {
		ps = (struct usb_port_status *)buf;
		ps->wPortStatus = 0;
		ps->wPortChange = 0;

		if (vhci->port_power_mask & BIT(wIndex))
			ps->wPortStatus |= USB_PORT_STAT_POWER;

		if (!(bce_vhci_port_mask & BIT(wIndex)))
			return 0;

		status = bce_vhci_cmd_port_status(&vhci->cq, (u8)wIndex, 0,
						  &port_status);
		if (status)
			return status;

		/*
		 * bridgeOS does not re-arm a preserved device's bulk OUT read credits
		 * across a stateful sleep; only the device's own SET_INTERFACE does.
		 * Report such a port as connected but not enabled until usbcore has
		 * reset it, which makes usbcore reset-resume the device (restore its
		 * configuration and interfaces) instead of resuming it in place.
		 * Devices with only IN endpoints keep resuming in place: their EP0
		 * credits do survive sleep.
		 */
		reset_resume =
			wIndex <= BITS_PER_LONG &&
			test_bit(wIndex - 1, &vhci->stateful_reset_ports);

		if ((port_status & BCE_VHCI_PORT_STATUS_ENABLED) &&
		    !reset_resume)
			ps->wPortStatus |= USB_PORT_STAT_ENABLE;
		if (port_status & BCE_VHCI_PORT_STATUS_CONNECTED)
			ps->wPortStatus |= USB_PORT_STAT_CONNECTION;
		if (port_status & BCE_VHCI_PORT_STATUS_OVERCURRENT)
			ps->wPortStatus |= USB_PORT_STAT_OVERCURRENT;
		if (port_status & BCE_VHCI_PORT_STATUS_RESET)
			ps->wPortStatus |= USB_PORT_STAT_RESET;
		if (port_status & BCE_VHCI_PORT_STATUS_HIGH_SPEED)
			ps->wPortStatus |= USB_PORT_STAT_HIGH_SPEED;
		if ((port_status & (BCE_VHCI_PORT_STATUS_SUSPENDED |
				    BCE_VHCI_PORT_STATUS_SUSPENDED_LEGACY)) &&
		    !reset_resume)
			ps->wPortStatus |= USB_PORT_STAT_SUSPEND;
		if (port_status & BCE_VHCI_PORT_STATUS_C_CONNECTION)
			ps->wPortChange |= USB_PORT_STAT_C_CONNECTION;
		if (reset_resume)
			pr_debug(
				"hub reporting port=%u for reset-resume raw=%x status=%x\n",
				wIndex, port_status, ps->wPortStatus);
		return 0;
	} else if (type_req == SetPortFeature) {
		if (wValue == USB_PORT_FEAT_POWER) {
			status = bce_vhci_cmd_port_power_on(&vhci->cq,
							    (u8)wIndex);
			/* usbcore tracks logical power state separately from bridgeOS. */
			if (!status)
				vhci->port_power_mask |= BIT(wIndex);
			return status;
		}
		if (wValue == USB_PORT_FEAT_RESET) {
			pr_debug("hub SetPortFeature RESET port=%u\n", wIndex);
			if (wIndex <= BITS_PER_LONG)
				clear_bit(wIndex - 1,
					  &vhci->stateful_reset_ports);
			return bce_vhci_reset_device(vhci, wIndex, wValue);
		}
		if (wValue == USB_PORT_FEAT_SUSPEND) {
			/* trace the usbcore hub state machine against the T2 port commands */
			pr_debug("hub SetPortFeature SUSPEND port=%u\n",
				 wIndex);
			if (vhci->system_suspending) {
				pr_debug(
					"hub SetPortFeature SUSPEND port=%u skipped during system suspend\n",
					wIndex);
				return 0;
			}
			status = bce_vhci_cmd_port_suspend(&vhci->cq,
							   (u8)wIndex);
			pr_debug("hub SetPortFeature SUSPEND port=%u -> %d\n",
				 wIndex, status);
			return status;
		}
	} else if (type_req == ClearPortFeature) {
		if (wValue == USB_PORT_FEAT_ENABLE)
			return bce_vhci_cmd_port_disable(&vhci->cq, (u8)wIndex);
		if (wValue == USB_PORT_FEAT_POWER) {
			status = bce_vhci_cmd_port_power_off(&vhci->cq,
							     (u8)wIndex);
			if (!status)
				vhci->port_power_mask &= ~BIT(wIndex);
			return status;
		}
		if (wValue == USB_PORT_FEAT_C_CONNECTION) {
			clear_bit(wIndex - 1, &vhci->port_change_pending);
			return bce_vhci_cmd_port_status(&vhci->cq, (u8)wIndex,
							0x40000, &port_status);
		}
		if (wValue == USB_PORT_FEAT_C_RESET)
			return 0;
		if (wValue == USB_PORT_FEAT_SUSPEND) {
			status =
				bce_vhci_cmd_port_resume(&vhci->cq, (u8)wIndex);
			pr_debug("hub ClearPortFeature SUSPEND port=%u -> %d\n",
				 wIndex, status);
			return status;
		}
	}
	pr_err("%s unhandled request: %x %i %i [bufl=%i]\n", __func__, type_req,
	       wValue, wIndex, wLength);
	dump_stack();
	return -EIO;
}

static int bce_vhci_enable_device(struct usb_hcd *hcd, struct usb_device *udev)
{
	struct bce_vhci *vhci = bce_vhci_from_hcd(hcd);
	struct bce_vhci_device *vdev;
	bce_vhci_device_t devid;

	pr_debug("enable_device port=%u\n", udev->portnum);

	if (vhci->port_to_device[udev->portnum])
		return 0;

	vdev = kzalloc_obj(*vdev);
	if (!vdev)
		return -ENOMEM;

	/* bridgeOS requires a firmware device id before endpoints are created. */
	if (bce_vhci_cmd_device_create(&vhci->cq, udev->portnum, &devid) ||
	    devid >= ARRAY_SIZE(vhci->devices)) {
		kfree(vdev);
		return -EIO;
	}

	pr_debug("device_create port=%u dev=%u\n", udev->portnum, devid);

	vhci->port_to_device[udev->portnum] = devid;
	vhci->devices[devid] = vdev;

	bce_vhci_create_transfer_queue(vhci, &vdev->tq[0], &udev->ep0, devid,
				       DMA_BIDIRECTIONAL);
	udev->ep0.hcpriv = &vdev->tq[0];
	vdev->tq_mask |= BIT(0);

	bce_vhci_cmd_endpoint_create(&vhci->cq, devid, &udev->ep0.desc);
	return 0;
}

static int bce_vhci_address_device(struct usb_hcd *hcd, struct usb_device *udev,
				   unsigned int timeout_ms)
{
	/*
	 * The firmware device is created here and from enable_device(). The
	 * firmware command path does not use the usbcore timeout.
	 */
	return bce_vhci_enable_device(hcd, udev);
}

static void bce_vhci_free_device(struct usb_hcd *hcd, struct usb_device *udev)
{
	struct bce_vhci *vhci = bce_vhci_from_hcd(hcd);
	int i;
	bce_vhci_device_t devid;
	struct bce_vhci_device *dev;

	pr_debug("free_device port=%u\n", udev->portnum);
	if (!vhci->port_to_device[udev->portnum])
		return;
	devid = vhci->port_to_device[udev->portnum];
	dev = vhci->devices[devid];
	for (i = 0; i < 32; i++) {
		if (dev->tq_mask & BIT(i)) {
			if (!bce_vhci_transfer_queue_pause(
				    &dev->tq[i], BCE_VHCI_PAUSE_SHUTDOWN))
				bce_vhci_cmd_endpoint_destroy(
					&vhci->cq, devid, dev->tq[i].endp_addr);
			else
				pr_warn("[%02x] pause not confirmed, skipping endpoint destroy\n",
					dev->tq[i].endp_addr);
			if (dev->tq[i].endp)
				dev->tq[i].endp->hcpriv = NULL;
			bce_vhci_destroy_transfer_queue(vhci, &dev->tq[i]);
		}
	}
	vhci->devices[devid] = NULL;
	vhci->port_to_device[udev->portnum] = 0;
	if (udev->portnum <= BITS_PER_LONG)
		clear_bit(udev->portnum - 1, &vhci->stateful_reset_ports);
	bce_vhci_cmd_device_destroy(&vhci->cq, devid);
	kfree(dev);
}

static int bce_vhci_reset_device(struct bce_vhci *vhci, int index, u16 timeout)
{
	struct bce_vhci_device *dev = NULL;
	bce_vhci_device_t devid;
	int i;
	int status;
	enum dma_data_direction dir;

	pr_debug("reset_device port=%d\n", index);

	devid = vhci->port_to_device[index];
	if (devid) {
		dev = vhci->devices[devid];

		for (i = 0; i < 32; i++) {
			if (dev->tq_mask & BIT(i)) {
				if (!bce_vhci_transfer_queue_pause(
					    &dev->tq[i],
					    BCE_VHCI_PAUSE_SHUTDOWN))
					bce_vhci_cmd_endpoint_destroy(
						&vhci->cq, devid,
						dev->tq[i].endp_addr);
				else
					pr_warn("[%02x] pause not confirmed, skipping endpoint destroy\n",
						dev->tq[i].endp_addr);
				if (dev->tq[i].endp)
					dev->tq[i].endp->hcpriv = NULL;
				bce_vhci_destroy_transfer_queue(vhci,
								&dev->tq[i]);
			}
		}
		vhci->devices[devid] = NULL;
		vhci->port_to_device[index] = 0;
		bce_vhci_cmd_device_destroy(&vhci->cq, devid);
	}
	status = bce_vhci_cmd_port_reset(&vhci->cq, (u8)index, timeout);

	if (dev) {
		status = bce_vhci_cmd_device_create(&vhci->cq, index, &devid);
		if (status)
			return status;
		if (devid >= ARRAY_SIZE(vhci->devices))
			return -EIO;
		vhci->devices[devid] = dev;
		vhci->port_to_device[index] = devid;

		for (i = 0; i < 32; i++) {
			if (dev->tq_mask & BIT(i)) {
				dir = usb_endpoint_dir_in(
					      &dev->tq[i].endp->desc) ?
					      DMA_FROM_DEVICE :
					      DMA_TO_DEVICE;
				if (i == 0)
					dir = DMA_BIDIRECTIONAL;
				bce_vhci_create_transfer_queue(vhci,
							       &dev->tq[i],
							       dev->tq[i].endp,
							       devid, dir);
				dev->tq[i].endp->hcpriv = &dev->tq[i];
				bce_vhci_cmd_endpoint_create(
					&vhci->cq, devid,
					&dev->tq[i].endp->desc);
			}
		}
	}

	return status;
}

static int bce_vhci_check_bandwidth(struct usb_hcd *hcd,
				    struct usb_device *udev)
{
	return 0;
}

static int bce_vhci_get_frame_number(struct usb_hcd *hcd)
{
	return 0;
}

static int bce_vhci_resume_suspended(struct bce_vhci *vhci)
{
	int devid, endp, port;
	int status = 0;
	int ret;

	for (port = 1; port < ARRAY_SIZE(vhci->port_to_device); port++) {
		if (!vhci->port_to_device[port])
			continue;
		ret = bce_vhci_cmd_port_resume(&vhci->cq, (u8)port);
		if (ret && !status)
			status = ret;
	}

	for (devid = 1; devid < ARRAY_SIZE(vhci->devices); devid++) {
		if (!vhci->devices[devid])
			continue;
		for (endp = 0; endp < 32; endp++) {
			if (!(vhci->devices[devid]->tq_mask & BIT(endp)))
				continue;
			ret = bce_vhci_transfer_queue_resume(
				&vhci->devices[devid]->tq[endp],
				BCE_VHCI_PAUSE_SUSPEND);
			if (ret && !status)
				status = ret;
		}
	}
	return status;
}

static int bce_vhci_suspend_quiesce(struct usb_hcd *hcd)
{
	int devid, endp, port;
	int status;
	struct bce_vhci *vhci = bce_vhci_from_hcd(hcd);

	pr_info("suspend started\n");
	flush_workqueue(vhci->tq_state_wq);

	for (devid = 1; devid < ARRAY_SIZE(vhci->devices); devid++) {
		if (!vhci->devices[devid])
			continue;
		for (endp = 0; endp < 32; endp++) {
			if (!(vhci->devices[devid]->tq_mask & BIT(endp)))
				continue;
			status = bce_vhci_transfer_queue_pause(
				&vhci->devices[devid]->tq[endp],
				BCE_VHCI_PAUSE_SUSPEND);
			if (status)
				goto abort;
		}
	}

	/*
	 * Only occupied virtual ports are suspended. bridgeOS rejects holes in
	 * the virtual hub, and that rejection must abort system PM.
	 */
	for (port = 1; port < ARRAY_SIZE(vhci->port_to_device); port++) {
		if (!vhci->port_to_device[port])
			continue;
		status = bce_vhci_cmd_port_suspend(&vhci->cq, (u8)port);
		if (status)
			goto abort;
	}

	pr_info("suspend quiesce complete\n");
	return 0;

abort:
	pr_err("suspend quiesce failed: %d\n", status);
	bce_vhci_resume_suspended(vhci);
	return status;
}

static int bce_vhci_bus_suspend(struct usb_hcd *hcd)
{
	int status;
	struct bce_vhci *vhci = bce_vhci_from_hcd(hcd);

	pr_info("bus_suspend entry\n");
	vhci->port_change_pending = 0;
	WRITE_ONCE(vhci->system_suspending, true);
	status = bce_vhci_suspend_quiesce(hcd);
	if (status)
		WRITE_ONCE(vhci->system_suspending, false);
	pr_info("bus_suspend exit status=%d\n", status);
	return status;
}

/*
 * Devices with bulk OUT endpoints lose the T2's queued reads across a
 * stateful sleep and need a reset-resume (see the hub GetPortStatus
 * handling). OUT endpoints other than EP0 use transfer queue indices 1
 * to 15.
 */
static void bce_vhci_mark_stateful_reset_ports(struct bce_vhci *vhci)
{
	int port;
	bce_vhci_device_t devid;

	vhci->stateful_reset_ports = 0;
	for (port = 1;
	     port < ARRAY_SIZE(vhci->port_to_device) && port <= BITS_PER_LONG;
	     port++) {
		devid = vhci->port_to_device[port];
		if (!devid || !vhci->devices[devid])
			continue;
		if (vhci->devices[devid]->tq_mask & GENMASK(15, 1)) {
			set_bit(port - 1, &vhci->stateful_reset_ports);
			pr_debug(
				"stateful resume will reset-resume port=%d dev=%u tq_mask=%x\n",
				port, devid, vhci->devices[devid]->tq_mask);
		}
	}
}

static int bce_vhci_bus_resume(struct usb_hcd *hcd)
{
	struct bce_vhci *vhci = bce_vhci_from_hcd(hcd);
	int status;

	pr_info("bus_resume entry\n");
	vhci->port_change_pending = 0;
	bce_vhci_mark_stateful_reset_ports(vhci);
	bce_vhci_resume_event_queues(vhci);
	status = bce_vhci_resume_suspended(vhci);
	WRITE_ONCE(vhci->system_suspending, false);
	pr_info("bus_resume exit status=%d\n", status);
	return status;
}

static int bce_vhci_urb_enqueue(struct usb_hcd *hcd, struct urb *urb,
				gfp_t mem_flags)
{
	struct bce_vhci *vhci = bce_vhci_from_hcd(hcd);
	struct bce_vhci_transfer_queue *q = urb->ep->hcpriv;

	if (READ_ONCE(vhci->system_suspending))
		return -ESHUTDOWN;
	if (!q)
		return -ENOENT;
	return bce_vhci_urb_create(q, urb, mem_flags);
}

static int bce_vhci_urb_dequeue(struct usb_hcd *hcd, struct urb *urb,
				int status)
{
	struct bce_vhci_transfer_queue *q = urb->ep->hcpriv;

	if (!q)
		return -ENOENT;
	return bce_vhci_urb_request_cancel(q, urb, status);
}

static void bce_vhci_endpoint_reset(struct usb_hcd *hcd,
				    struct usb_host_endpoint *ep)
{
	struct bce_vhci_transfer_queue *q = ep->hcpriv;

	pr_debug("endpoint_reset\n");
	if (q)
		bce_vhci_transfer_queue_request_reset(q);
}

static u8 bce_vhci_endpoint_index(u8 addr)
{
	if (addr & 0x80)
		return (u8)(0x10 + (addr & 0xf));
	return (u8)(addr & 0xf);
}

static int bce_vhci_add_endpoint(struct usb_hcd *hcd, struct usb_device *udev,
				 struct usb_host_endpoint *endp)
{
	u8 endp_index = bce_vhci_endpoint_index(endp->desc.bEndpointAddress);
	struct bce_vhci *vhci = bce_vhci_from_hcd(hcd);
	bce_vhci_device_t devid = vhci->port_to_device[udev->portnum];
	struct bce_vhci_device *vdev = vhci->devices[devid];

	pr_debug("add_endpoint port=%u dev=%u ep_index=%u\n", udev->portnum,
		 devid, endp_index);

	if (udev->bus->root_hub == udev) /* The USB hub */
		return 0;
	if (!vdev)
		return -ENODEV;
	if (vdev->tq_mask & BIT(endp_index)) {
		endp->hcpriv = &vdev->tq[endp_index];
		return 0;
	}

	bce_vhci_create_transfer_queue(vhci, &vdev->tq[endp_index], endp, devid,
				       usb_endpoint_dir_in(&endp->desc) ?
					       DMA_FROM_DEVICE :
					       DMA_TO_DEVICE);
	endp->hcpriv = &vdev->tq[endp_index];
	vdev->tq_mask |= BIT(endp_index);

	bce_vhci_cmd_endpoint_create(&vhci->cq, devid, &endp->desc);
	return 0;
}

static int bce_vhci_drop_endpoint(struct usb_hcd *hcd, struct usb_device *udev,
				  struct usb_host_endpoint *endp)
{
	u8 endp_index = bce_vhci_endpoint_index(endp->desc.bEndpointAddress);
	struct bce_vhci *vhci = bce_vhci_from_hcd(hcd);
	bce_vhci_device_t devid = vhci->port_to_device[udev->portnum];
	struct bce_vhci_transfer_queue *q = endp->hcpriv;
	struct bce_vhci_device *vdev;

	pr_debug("drop_endpoint port=%u ep_index=%u\n", udev->portnum,
		 endp_index);
	if (!devid || !vhci->devices[devid]) {
		endp->hcpriv = NULL;
		return 0;
	}
	vdev = vhci->devices[devid];
	if (!q) {
		if (vdev->tq_mask & BIT(endp_index)) {
			pr_err("endpoint missing hcpriv while still tracked\n");
			q = &vdev->tq[endp_index];
		} else {
			return 0;
		}
	}

	if (!bce_vhci_transfer_queue_pause(q, BCE_VHCI_PAUSE_SHUTDOWN))
		bce_vhci_cmd_endpoint_destroy(&vhci->cq, devid,
					      (u8)(endp->desc.bEndpointAddress &
						   0x8Fu));
	else
		pr_warn("[%02x] pause not confirmed, skipping endpoint destroy\n",
			(u8)(endp->desc.bEndpointAddress & 0x8Fu));
	vdev->tq_mask &= ~BIT(endp_index);
	bce_vhci_destroy_transfer_queue(vhci, q);
	endp->hcpriv = NULL;
	return 0;
}

static int bce_vhci_create_message_queues(struct bce_vhci *vhci)
{
	if (bce_vhci_message_queue_create(vhci, &vhci->msg_commands,
					  "VHC1HostCommands") ||
	    bce_vhci_message_queue_create(vhci, &vhci->msg_system,
					  "VHC1HostSystemEvents") ||
	    bce_vhci_message_queue_create(vhci, &vhci->msg_isochronous,
					  "VHC1HostIsochronousEvents") ||
	    bce_vhci_message_queue_create(vhci, &vhci->msg_interrupt,
					  "VHC1HostInterruptEvents") ||
	    bce_vhci_message_queue_create(vhci, &vhci->msg_asynchronous,
					  "VHC1HostAsynchronousEvents")) {
		bce_vhci_destroy_message_queues(vhci);
		return -EINVAL;
	}
	spin_lock_init(&vhci->msg_asynchronous_lock);
	bce_vhci_command_queue_create(&vhci->cq, &vhci->msg_commands);
	return 0;
}

static void bce_vhci_destroy_message_queues(struct bce_vhci *vhci)
{
	bce_vhci_command_queue_destroy(&vhci->cq);
	bce_vhci_message_queue_destroy(vhci, &vhci->msg_commands);
	bce_vhci_message_queue_destroy(vhci, &vhci->msg_system);
	bce_vhci_message_queue_destroy(vhci, &vhci->msg_isochronous);
	bce_vhci_message_queue_destroy(vhci, &vhci->msg_interrupt);
	bce_vhci_message_queue_destroy(vhci, &vhci->msg_asynchronous);
}

static void bce_vhci_handle_system_event(struct bce_vhci_event_queue *q,
					 struct bce_vhci_message *msg);
static void bce_vhci_handle_usb_event(struct bce_vhci_event_queue *q,
				      struct bce_vhci_message *msg);

static int bce_vhci_create_event_queues(struct bce_vhci *vhci)
{
	vhci->ev_cq = apple_bce_cq_create(vhci->bce, 0x100);
	if (!vhci->ev_cq)
		return -EINVAL;
	if (__bce_vhci_event_queue_create(vhci, &vhci->ev_commands,
					  "VHC1FirmwareCommands",
					  bce_vhci_firmware_event_completion) ||
	    bce_vhci_event_queue_create(vhci, &vhci->ev_system,
					"VHC1FirmwareSystemEvents",
					bce_vhci_handle_system_event) ||
	    bce_vhci_event_queue_create(vhci, &vhci->ev_isochronous,
					"VHC1FirmwareIsochronousEvents",
					bce_vhci_handle_usb_event) ||
	    bce_vhci_event_queue_create(vhci, &vhci->ev_interrupt,
					"VHC1FirmwareInterruptEvents",
					bce_vhci_handle_usb_event) ||
	    bce_vhci_event_queue_create(vhci, &vhci->ev_asynchronous,
					"VHC1FirmwareAsynchronousEvents",
					bce_vhci_handle_usb_event)) {
		bce_vhci_destroy_event_queues(vhci);
		return -EINVAL;
	}
	return 0;
}

static void bce_vhci_destroy_event_queues(struct bce_vhci *vhci)
{
	bce_vhci_event_queue_destroy(vhci, &vhci->ev_commands);
	bce_vhci_event_queue_destroy(vhci, &vhci->ev_system);
	bce_vhci_event_queue_destroy(vhci, &vhci->ev_isochronous);
	bce_vhci_event_queue_destroy(vhci, &vhci->ev_interrupt);
	bce_vhci_event_queue_destroy(vhci, &vhci->ev_asynchronous);
	if (vhci->ev_cq)
		apple_bce_cq_destroy(vhci->ev_cq);
}

static void bce_vhci_send_fw_event_response(struct bce_vhci *vhci,
					    struct bce_vhci_message *req,
					    u16 status)
{
	unsigned long timeout = 1000;
	struct bce_vhci_message r = *req;

	r.cmd = (u16)(req->cmd | 0x8000u);
	r.status = status;
	r.param1 = req->param1;
	r.param2 = 0;

	if (apple_bce_sq_reserve(vhci->msg_system.sq, &timeout)) {
		pr_err("Cannot reserve submision for FW event reply\n");
		return;
	}
	bce_vhci_message_queue_write(&vhci->msg_system, &r);
}

static int bce_vhci_handle_firmware_event(struct bce_vhci *vhci,
					  struct bce_vhci_message *msg)
{
	unsigned long flags;
	bce_vhci_device_t devid;
	u8 endp;
	struct bce_vhci_device *dev;
	struct bce_vhci_transfer_queue *tq;

	if (msg->cmd == BCE_VHCI_CMD_ENDPOINT_REQUEST_STATE ||
	    msg->cmd == BCE_VHCI_CMD_ENDPOINT_SET_STATE) {
		devid = (bce_vhci_device_t)(msg->param1 & 0xff);
		endp = bce_vhci_endpoint_index((u8)((msg->param1 >> 8) & 0xff));
		if (devid >= ARRAY_SIZE(vhci->devices))
			return BCE_VHCI_BAD_ARGUMENT;
		dev = vhci->devices[devid];
		if (!dev || !(dev->tq_mask & BIT(endp)))
			return BCE_VHCI_BAD_ARGUMENT;
		tq = &dev->tq[endp];
	}

	if (msg->cmd == BCE_VHCI_CMD_ENDPOINT_REQUEST_STATE) {
		if (msg->param2 == BCE_VHCI_ENDPOINT_ACTIVE) {
			int ret = bce_vhci_transfer_queue_resume(
				tq, BCE_VHCI_PAUSE_FIRMWARE);

			pr_debug(
				"FW endpoint request ACTIVE dev=%u ep=%02x paused_by=%x state=%x active=%u ret=%d reply=%u\n",
				devid, tq->endp_addr, tq->paused_by, tq->state,
				tq->active, ret,
				ret ? BCE_VHCI_ERROR : BCE_VHCI_SUCCESS);
			return ret ? BCE_VHCI_ERROR : BCE_VHCI_SUCCESS;
		} else if (msg->param2 == BCE_VHCI_ENDPOINT_PAUSED) {
			int ret = bce_vhci_transfer_queue_pause(
				tq, BCE_VHCI_PAUSE_FIRMWARE);

			pr_debug(
				"FW endpoint request PAUSED dev=%u ep=%02x paused_by=%x state=%x active=%u ret=%d reply=%u\n",
				devid, tq->endp_addr, tq->paused_by, tq->state,
				tq->active, ret,
				ret ? BCE_VHCI_ERROR : BCE_VHCI_SUCCESS);
			return ret ? BCE_VHCI_ERROR : BCE_VHCI_SUCCESS;
		}
		return BCE_VHCI_BAD_ARGUMENT;
	} else if (msg->cmd == BCE_VHCI_CMD_ENDPOINT_SET_STATE) {
		if (msg->param2 == BCE_VHCI_ENDPOINT_STALLED) {
			tq->state = msg->param2;
			spin_lock_irqsave(&tq->urb_lock, flags);
			tq->active = false;
			tq->stalled = true;
			spin_unlock_irqrestore(&tq->urb_lock, flags);
			pr_warn("firmware endpoint stalled dev=%u ep=%02x paused_by=%x; %s\n",
				devid, tq->endp_addr, tq->paused_by,
				tq->endp_addr == 0x00 ? "scheduling EP0 reset" :
							"leaving stalled");
			if (tq->endp_addr == 0x00)
				bce_vhci_transfer_queue_request_reset(tq);
			return BCE_VHCI_SUCCESS;
		}
		return BCE_VHCI_BAD_ARGUMENT;
	}
	pr_warn("Unhandled firmware event: %x s=%x p1=%x p2=%llx\n", msg->cmd,
		msg->status, msg->param1, msg->param2);
	return BCE_VHCI_BAD_ARGUMENT;
}

static void bce_vhci_handle_firmware_events_w(struct work_struct *ws)
{
	size_t cnt = 0;
	int result;
	struct bce_vhci *vhci = container_of(ws, struct bce_vhci, w_fw_events);
	struct apple_bce_sq *sq = vhci->ev_commands.sq;
	struct apple_bce_completion *cq;
	struct bce_vhci_message *msg, *msg2 = NULL;

	while (true) {
		if (msg2) {
			msg = msg2;
			msg2 = NULL;
		} else {
			cq = apple_bce_sq_next_completion(sq);
			if (!cq)
				break;
			if (cq->status == APPLE_BCE_STATUS_ABORTED) {
				apple_bce_sq_complete(sq);
				continue;
			}
			msg = &vhci->ev_commands.data[apple_bce_sq_head(sq)];
		}

		pr_debug("Got fw event: %x s=%x p1=%x p2=%llx\n", msg->cmd,
			 msg->status, msg->param1, msg->param2);
		cq = apple_bce_sq_next_completion(sq);
		if (cq) {
			msg2 = &vhci->ev_commands
					.data[(apple_bce_sq_head(sq) + 1) %
					      apple_bce_sq_size(sq)];
			pr_debug("Got second fw event: %x s=%x p1=%x p2=%llx\n",
				 msg->cmd, msg->status, msg->param1,
				 msg->param2);
			if (cq->status != APPLE_BCE_STATUS_ABORTED &&
			    msg2->cmd == (msg->cmd | 0x4000) &&
			    msg2->param1 == msg->param1) {
				/* Take two elements */
				pr_debug("Cancelled\n");
				bce_vhci_send_fw_event_response(vhci, msg,
								BCE_VHCI_ABORT);

				apple_bce_sq_complete(sq);
				apple_bce_sq_complete(sq);
				msg2 = NULL;
				cnt += 2;
				continue;
			}

			pr_warn("Handle fw event - unexpected cancellation\n");
		}

		result = bce_vhci_handle_firmware_event(vhci, msg);
		bce_vhci_send_fw_event_response(vhci, msg, (u16)result);

		apple_bce_sq_complete(sq);
		++cnt;
	}
	bce_vhci_event_queue_submit_pending(&vhci->ev_commands, cnt);
	if (apple_bce_sq_idle(sq)) {
		pr_debug("complete\n");
		complete(&vhci->ev_commands.queue_empty_completion);
	}
}

static void bce_vhci_firmware_event_completion(struct apple_bce_sq *sq)
{
	struct bce_vhci_event_queue *q = apple_bce_sq_priv(sq);

	queue_work(q->vhci->tq_state_wq, &q->vhci->w_fw_events);
}

static const char *bce_vhci_port_state_change_label(u64 p2)
{
	return (p2 & 0x4) ? "connected" : "disconnected";
}

static void bce_vhci_handle_system_event(struct bce_vhci_event_queue *q,
					 struct bce_vhci_message *msg)
{
	if (msg->cmd & 0x8000) {
		bce_vhci_command_queue_deliver_completion(&q->vhci->cq, msg);
	} else if (msg->cmd == BCE_VHCI_EV_PORT_STATUS_CHANGE &&
		   msg->param1 > 0 &&
		   msg->param1 < ARRAY_SIZE(q->vhci->port_to_device)) {
		set_bit(msg->param1 - 1, &q->vhci->port_change_pending);
		pr_debug(
			"port state change event: port=%u state=%s status=%x p2=%llx\n",
			msg->param1,
			bce_vhci_port_state_change_label(msg->param2),
			msg->status, msg->param2);
	} else {
		pr_warn("Unhandled system event: %x s=%x p1=%x p2=%llx\n",
			msg->cmd, msg->status, msg->param1, msg->param2);
	}
}

static void bce_vhci_handle_usb_event(struct bce_vhci_event_queue *q,
				      struct bce_vhci_message *msg)
{
	bce_vhci_device_t devid;
	u8 endp;
	struct bce_vhci_device *dev;

	if (msg->cmd & 0x8000) {
		bce_vhci_command_queue_deliver_completion(&q->vhci->cq, msg);
	} else if (msg->cmd == BCE_VHCI_CMD_TRANSFER_REQUEST ||
		   msg->cmd == BCE_VHCI_CMD_CONTROL_TRANSFER_STATUS) {
		devid = (bce_vhci_device_t)(msg->param1 & 0xff);
		endp = bce_vhci_endpoint_index((u8)((msg->param1 >> 8) & 0xff));
		dev = devid < ARRAY_SIZE(q->vhci->devices) ?
			      q->vhci->devices[devid] :
			      NULL;
		if (!dev || (dev->tq_mask & BIT(endp)) == 0) {
			pr_err("Didn't find destination for transfer queue event\n");
			return;
		}
		bce_vhci_transfer_queue_event(&dev->tq[endp], msg);
	} else {
		pr_warn("Unhandled USB event: %x s=%x p1=%x p2=%llx\n",
			msg->cmd, msg->status, msg->param1, msg->param2);
	}
}

static const struct hc_driver bce_vhci_driver = {
	.description = "apple-t2-vhci",
	.product_desc = "BCE VHCI Host Controller",
	.hcd_priv_size = sizeof(struct bce_vhci *),

	.flags = HCD_USB2 | HCD_DMA,

	.start = bce_vhci_start,
	.stop = bce_vhci_stop,
	.hub_status_data = bce_vhci_hub_status_data,
	.hub_control = bce_vhci_hub_control,
	.urb_enqueue = bce_vhci_urb_enqueue,
	.urb_dequeue = bce_vhci_urb_dequeue,
	.enable_device = bce_vhci_enable_device,
	.free_dev = bce_vhci_free_device,
	.address_device = bce_vhci_address_device,
	.add_endpoint = bce_vhci_add_endpoint,
	.drop_endpoint = bce_vhci_drop_endpoint,
	.endpoint_reset = bce_vhci_endpoint_reset,
	.check_bandwidth = bce_vhci_check_bandwidth,
	.get_frame_number = bce_vhci_get_frame_number,
	.bus_suspend = bce_vhci_bus_suspend,
	.bus_resume = bce_vhci_bus_resume
};

/* usb_create_hcd() sets the drvdata of the controller device to the HCD */
static struct bce_vhci *bce_vhci_from_dev(struct device *dev)
{
	return bce_vhci_from_hcd(dev_get_drvdata(dev));
}

static int bce_vhci_probe(struct auxiliary_device *adev,
			  const struct auxiliary_device_id *id)
{
	struct bce_vhci *vhci;

	vhci = devm_kzalloc(&adev->dev, sizeof(*vhci), GFP_KERNEL);
	if (!vhci)
		return -ENOMEM;
	vhci->bce = dev_get_platdata(&adev->dev);
	if (!vhci->bce)
		return -ENODEV;
	return bce_vhci_create(&adev->dev, vhci);
}

static void bce_vhci_remove(struct auxiliary_device *adev)
{
	bce_vhci_destroy(bce_vhci_from_dev(&adev->dev));
}

static void bce_vhci_aux_shutdown(struct auxiliary_device *adev)
{
	bce_vhci_shutdown(bce_vhci_from_dev(&adev->dev));
}

static int bce_vhci_suspend(struct device *dev)
{
	return bce_vhci_pm_prepare(bce_vhci_from_dev(dev));
}

static const struct dev_pm_ops bce_vhci_pm_ops = {
	.suspend = bce_vhci_suspend,
};

static const struct auxiliary_device_id bce_vhci_ids[] = {
	{ .name = "apple_t2_bridge.vhci" },
	{}
};
MODULE_DEVICE_TABLE(auxiliary, bce_vhci_ids);

static struct auxiliary_driver bce_vhci_aux_driver = {
	.name = "vhci",
	.id_table = bce_vhci_ids,
	.probe = bce_vhci_probe,
	.remove = bce_vhci_remove,
	.shutdown = bce_vhci_aux_shutdown,
	.driver.pm = pm_sleep_ptr(&bce_vhci_pm_ops),
};
module_auxiliary_driver(bce_vhci_aux_driver);

module_param_named(vhci_port_mask, bce_vhci_port_mask, ushort, 0444);
MODULE_PARM_DESC(vhci_port_mask, "Specifies which VHCI ports are enabled");
MODULE_IMPORT_NS("APPLE_T2_BRIDGE");
MODULE_AUTHOR("Paul Pawlowski <paul@mrarm.io>");
MODULE_AUTHOR("André Eikmeyer <andre.eikmeyer@kait2en.org>");
MODULE_DESCRIPTION("Apple T2 virtual USB host controller");
MODULE_LICENSE("GPL");
