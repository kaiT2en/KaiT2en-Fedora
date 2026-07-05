#ifndef T2BCE_TRANSPORT_H
#define T2BCE_TRANSPORT_H

#include <linux/device.h>
#include "../queue.h"

struct aaudio_device;
struct t2bce_client;

struct t2bce_client *t2bce_client_get(struct device *dev);
void t2bce_client_put(struct t2bce_client *client);

struct device *t2bce_client_dma_dev(struct t2bce_client *client);
bool t2bce_client_no_state_resume(struct t2bce_client *client);

void t2bce_client_set_audio(struct t2bce_client *client, struct aaudio_device *audio);
void t2bce_client_clear_audio(struct t2bce_client *client, struct aaudio_device *audio);

struct bce_queue_cq *t2bce_create_cq(struct t2bce_client *client, u32 el_count);
void t2bce_destroy_cq(struct t2bce_client *client, struct bce_queue_cq *cq);

struct bce_queue_sq *t2bce_create_sq(struct t2bce_client *client, struct bce_queue_cq *cq,
        const char *name, u32 el_count, int direction, bce_sq_completion compl, void *userdata);
void t2bce_destroy_sq(struct t2bce_client *client, struct bce_queue_sq *sq);

#endif
