#include "t2bce_transport.h"
#include "t2bce.h"

#include <linux/err.h>
#include <linux/export.h>
#include <linux/slab.h>

struct t2bce_client {
    struct t2bce_device *bce;
    struct device *dev;
    struct device_link *link;
    struct list_head list;
    t2bce_resume_callback post_vhci_resume;
    void *post_vhci_resume_userdata;
};

struct t2bce_sq_ctx {
    t2bce_sq_completion completion;
    void *userdata;
};

static struct bce_queue_cq *to_bce_cq(struct t2bce_queue_cq *cq)
{
    return (struct bce_queue_cq *) cq;
}

static struct t2bce_queue_cq *to_t2bce_cq(struct bce_queue_cq *cq)
{
    return (struct t2bce_queue_cq *) cq;
}

static struct bce_queue_sq *to_bce_sq(struct t2bce_queue_sq *sq)
{
    return (struct bce_queue_sq *) sq;
}

static struct t2bce_queue_sq *to_t2bce_sq(struct bce_queue_sq *sq)
{
    return (struct t2bce_queue_sq *) sq;
}

static void t2bce_sq_completion_adapter(struct bce_queue_sq *sq)
{
    struct t2bce_sq_ctx *ctx = sq->userdata;

    ctx->completion(to_t2bce_sq(sq));
}

struct t2bce_client *t2bce_client_get(struct device *dev)
{
    struct t2bce_device *bce = global_bce;
    struct t2bce_client *client;

    if (!bce)
        return ERR_PTR(-EPROBE_DEFER);

    if (bce->is_being_removed)
        return ERR_PTR(-ENODEV);

    client = kzalloc(sizeof(*client), GFP_KERNEL);
    if (!client)
        return ERR_PTR(-ENOMEM);

    client->bce = bce;
    client->dev = dev;
    client->link = device_link_add(dev, &bce->pci->dev,
            DL_FLAG_PM_RUNTIME | DL_FLAG_AUTOREMOVE_CONSUMER);
    if (!client->link) {
        kfree(client);
        return ERR_PTR(-ENODEV);
    }

    mutex_lock(&bce->clients_lock);
    list_add_tail_rcu(&client->list, &bce->clients);
    mutex_unlock(&bce->clients_lock);

    return client;
}
EXPORT_SYMBOL_GPL(t2bce_client_get);

void t2bce_client_put(struct t2bce_client *client)
{
    if (!client)
        return;

    mutex_lock(&client->bce->clients_lock);
    list_del_rcu(&client->list);
    mutex_unlock(&client->bce->clients_lock);

    synchronize_srcu(&client->bce->clients_srcu);

    if (client->link)
        device_link_del(client->link);
    kfree(client);
}
EXPORT_SYMBOL_GPL(t2bce_client_put);

struct device *t2bce_client_dma_dev(struct t2bce_client *client)
{
    return &client->bce->pci->dev;
}
EXPORT_SYMBOL_GPL(t2bce_client_dma_dev);

bool t2bce_client_no_state_resume(struct t2bce_client *client)
{
    return client->bce->vhci.no_state_resume;
}
EXPORT_SYMBOL_GPL(t2bce_client_no_state_resume);

void t2bce_client_set_post_vhci_resume(struct t2bce_client *client,
        t2bce_resume_callback callback, void *userdata)
{
    if (!callback) {
        smp_store_release(&client->post_vhci_resume, NULL);
        WRITE_ONCE(client->post_vhci_resume_userdata, NULL);
        return;
    }

    WRITE_ONCE(client->post_vhci_resume_userdata, userdata);
    smp_store_release(&client->post_vhci_resume, callback);
}
EXPORT_SYMBOL_GPL(t2bce_client_set_post_vhci_resume);

void t2bce_notify_post_vhci_resume(struct t2bce_device *bce)
{
    struct t2bce_client *client;
    int srcu_idx;

    srcu_idx = srcu_read_lock(&bce->clients_srcu);
    list_for_each_entry_srcu(client, &bce->clients, list,
            srcu_read_lock_held(&bce->clients_srcu)) {
        t2bce_resume_callback callback = smp_load_acquire(&client->post_vhci_resume);

        if (callback)
            callback(READ_ONCE(client->post_vhci_resume_userdata));
    }
    srcu_read_unlock(&bce->clients_srcu, srcu_idx);
}

struct t2bce_queue_cq *t2bce_create_cq(struct t2bce_client *client, u32 el_count)
{
    return to_t2bce_cq(bce_create_cq(client->bce, el_count));
}
EXPORT_SYMBOL_GPL(t2bce_create_cq);

void t2bce_destroy_cq(struct t2bce_client *client, struct t2bce_queue_cq *cq)
{
    bce_destroy_cq(client->bce, to_bce_cq(cq));
}
EXPORT_SYMBOL_GPL(t2bce_destroy_cq);

struct t2bce_queue_sq *t2bce_create_sq(struct t2bce_client *client, struct t2bce_queue_cq *cq,
        const char *name, u32 el_count, enum dma_data_direction direction,
        t2bce_sq_completion compl, void *userdata)
{
    struct t2bce_sq_ctx *ctx;
    struct bce_queue_sq *sq;

    ctx = kzalloc(sizeof(*ctx), GFP_KERNEL);
    if (!ctx)
        return NULL;

    ctx->completion = compl;
    ctx->userdata = userdata;

    sq = bce_create_sq(client->bce, to_bce_cq(cq), name, el_count, direction,
            t2bce_sq_completion_adapter, ctx);
    if (!sq) {
        kfree(ctx);
        return NULL;
    }

    return to_t2bce_sq(sq);
}
EXPORT_SYMBOL_GPL(t2bce_create_sq);

void t2bce_destroy_sq(struct t2bce_client *client, struct t2bce_queue_sq *sq)
{
    struct bce_queue_sq *bce_sq = to_bce_sq(sq);
    struct t2bce_sq_ctx *ctx = bce_sq->userdata;

    bce_destroy_sq(client->bce, bce_sq);
    kfree(ctx);
}
EXPORT_SYMBOL_GPL(t2bce_destroy_sq);

void *t2bce_queue_sq_userdata(struct t2bce_queue_sq *sq)
{
    struct t2bce_sq_ctx *ctx = to_bce_sq(sq)->userdata;

    return ctx->userdata;
}
EXPORT_SYMBOL_GPL(t2bce_queue_sq_userdata);

int t2bce_reserve_submission(struct t2bce_queue_sq *sq, unsigned long *timeout)
{
    return bce_reserve_submission(to_bce_sq(sq), timeout);
}
EXPORT_SYMBOL_GPL(t2bce_reserve_submission);

void t2bce_set_next_submission_single(struct t2bce_queue_sq *sq, dma_addr_t addr, size_t size)
{
    struct bce_qe_submission *submission = bce_next_submission(to_bce_sq(sq));

    bce_set_submission_single(submission, addr, size);
}
EXPORT_SYMBOL_GPL(t2bce_set_next_submission_single);

void t2bce_submit_to_device(struct t2bce_queue_sq *sq)
{
    bce_submit_to_device(to_bce_sq(sq));
}
EXPORT_SYMBOL_GPL(t2bce_submit_to_device);

void t2bce_notify_submission_complete(struct t2bce_queue_sq *sq)
{
    bce_notify_submission_complete(to_bce_sq(sq));
}
EXPORT_SYMBOL_GPL(t2bce_notify_submission_complete);

struct t2bce_sq_completion_data *t2bce_next_completion(struct t2bce_queue_sq *sq)
{
    return (struct t2bce_sq_completion_data *) bce_next_completion(to_bce_sq(sq));
}
EXPORT_SYMBOL_GPL(t2bce_next_completion);
