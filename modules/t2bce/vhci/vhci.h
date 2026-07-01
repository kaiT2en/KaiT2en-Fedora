#ifndef BCE_VHCI_H
#define BCE_VHCI_H

#include "queue.h"
#include "transfer.h"

#define BCE_VHCI_EP0_HOLD_TIMEOUT_MS 5000

struct usb_hcd;
struct bce_queue_cq;

struct bce_vhci_device {
    struct bce_vhci_transfer_queue tq[32];
    u32 tq_mask;
};
struct bce_vhci {
    struct t2bce_device *dev;
    dev_t vdevt;
    struct device *vdev;
    struct usb_hcd *hcd;
    struct spinlock hcd_spinlock;
    struct bce_vhci_message_queue msg_commands;
    struct bce_vhci_message_queue msg_system;
    struct bce_vhci_message_queue msg_isochronous;
    struct bce_vhci_message_queue msg_interrupt;
    struct bce_vhci_message_queue msg_asynchronous;
    struct spinlock msg_asynchronous_lock;
    struct bce_vhci_command_queue cq;
    struct bce_queue_cq *ev_cq;
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
    struct work_struct w_add_hcd;
    struct delayed_work w_port_status_change;
    unsigned long port_change_pending;
    unsigned long port_change_waiting;
    /* Worker-controlled EP0 gate; cleared again when EP0 reports status=3. */
    unsigned long port_ep0_ready;
    /* jiffies when the first EP0 URB was held, per port; 0 = no URB held */
    unsigned long port_ep0_hold_start[17];
    u8 port_resume_tries[17];
    bool no_state_resume;
    bool hcd_registered;
    bool system_suspending;
};

bool bce_vhci_port_status_ready_for_traffic(u32 port_status);

int __init bce_vhci_module_init(void);
void __exit bce_vhci_module_exit(void);

int bce_vhci_create(struct t2bce_device *dev, struct bce_vhci *vhci);
void bce_vhci_destroy(struct bce_vhci *vhci);
int bce_vhci_start(struct usb_hcd *hcd);
void bce_vhci_stop(struct usb_hcd *hcd);
int bce_vhci_add_hcd(struct bce_vhci *vhci);
void bce_vhci_remove_hcd(struct bce_vhci *vhci);

struct bce_vhci *bce_vhci_from_hcd(struct usb_hcd *hcd);

#endif //BCE_VHCI_H
