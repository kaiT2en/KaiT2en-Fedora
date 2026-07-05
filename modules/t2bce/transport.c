#include "t2bce_transport.h"
#include "t2bce.h"

#include <linux/slab.h>

struct t2bce_client {
    struct t2bce_device *bce;
    struct device *dev;
    struct device_link *link;
};

struct t2bce_client *t2bce_client_get(struct device *dev)
{
    struct t2bce_client *client;

    if (!global_bce)
        return NULL;

    client = kzalloc(sizeof(*client), GFP_KERNEL);
    if (!client)
        return NULL;

    client->bce = global_bce;
    client->dev = dev;
    client->link = device_link_add(dev, &global_bce->pci->dev,
            DL_FLAG_PM_RUNTIME | DL_FLAG_AUTOREMOVE_CONSUMER);
    return client;
}

void t2bce_client_put(struct t2bce_client *client)
{
    if (!client)
        return;

    if (client->link)
        device_link_del(client->link);
    kfree(client);
}

struct device *t2bce_client_dma_dev(struct t2bce_client *client)
{
    return &client->bce->pci->dev;
}

bool t2bce_client_no_state_resume(struct t2bce_client *client)
{
    return client->bce->vhci.no_state_resume;
}

void t2bce_client_set_audio(struct t2bce_client *client, struct aaudio_device *audio)
{
    client->bce->aaudio = audio;
}

void t2bce_client_clear_audio(struct t2bce_client *client, struct aaudio_device *audio)
{
    if (client && client->bce->aaudio == audio)
        client->bce->aaudio = NULL;
}

struct bce_queue_cq *t2bce_create_cq(struct t2bce_client *client, u32 el_count)
{
    return bce_create_cq(client->bce, el_count);
}

void t2bce_destroy_cq(struct t2bce_client *client, struct bce_queue_cq *cq)
{
    bce_destroy_cq(client->bce, cq);
}

struct bce_queue_sq *t2bce_create_sq(struct t2bce_client *client, struct bce_queue_cq *cq,
        const char *name, u32 el_count, int direction, bce_sq_completion compl, void *userdata)
{
    return bce_create_sq(client->bce, cq, name, el_count, direction, compl, userdata);
}

void t2bce_destroy_sq(struct t2bce_client *client, struct bce_queue_sq *sq)
{
    bce_destroy_sq(client->bce, sq);
}
