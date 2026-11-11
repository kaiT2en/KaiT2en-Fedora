// SPDX-License-Identifier: GPL-2.0
/*
 * Apple T2 audio
 *
 * The audio function of the T2 holds the sample buffers in BAR 0. A
 * descriptor of all devices, streams and buffers (the BufferStruct) lives in
 * the same BAR, its offset is published in the general purpose registers
 * (GPR) at BAR 4 + 0xc000. Control traffic runs over two queues of the
 * Apple T2 Bridge Controller.
 *
 * Copyright (C) Paul Pawlowski <paul@mrarm.io>
 * Copyright (C) André Eikmeyer <andre.eikmeyer@kait2en.org>
 */

#define pr_fmt(fmt) KBUILD_MODNAME ": " fmt

#include <linux/debugfs.h>
#include <linux/err.h>
#include <linux/module.h>
#include <linux/pci.h>
#include <linux/spinlock.h>
#include <linux/uaccess.h>
#include <sound/core.h>
#include <sound/initval.h>
#include <sound/jack.h>
#include <sound/pcm.h>

#include "audio.h"
#include "pcm.h"
#include "timing.h"

#define T2AUDIO_GPR_OFFSET 0xc000

static int t2audio_alsa_index = SNDRV_DEFAULT_IDX1;
static char *t2audio_alsa_id = SNDRV_DEFAULT_STR1;

static int t2audio_init_cmd(struct t2audio_device *a);
static int t2audio_init_bs(struct t2audio_device *a);
static int t2audio_reset_buffers(struct t2audio_device *a);
static void t2audio_init_dev(struct t2audio_device *a,
			     t2audio_device_id_t dev_id);
static void t2audio_free_dev(struct t2audio_subdevice *sdev);
static void t2audio_reset_stream(struct t2audio_stream *stream);
static void t2audio_reset_streams(struct t2audio_device *a);
static void t2audio_resume_work(struct work_struct *ws);
static int t2audio_debugfs_init(struct t2audio_device *a);

static int t2audio_probe(struct pci_dev *dev, const struct pci_device_id *id)
{
	struct t2audio_device *t2audio = NULL;
	struct t2audio_subdevice *sdev = NULL;
	int status = 0;
	u32 cfg;

	if (pci_enable_device(dev))
		return -ENODEV;
	if (pci_request_regions(dev, KBUILD_MODNAME)) {
		status = -ENODEV;
		goto fail;
	}
	pci_set_master(dev);

	t2audio = kzalloc_obj(*t2audio);
	if (!t2audio) {
		status = -ENOMEM;
		goto fail;
	}

	t2audio->bce = apple_bce_get(&dev->dev);
	if (IS_ERR(t2audio->bce)) {
		status = dev_err_probe(&dev->dev, PTR_ERR(t2audio->bce),
				       "bridge not available\n");
		t2audio->bce = NULL;
		goto fail;
	}

	t2audio->pci = dev;
	t2audio->dev = &dev->dev;
	pci_set_drvdata(dev, t2audio);

	init_completion(&t2audio->remote_alive);
	INIT_WORK(&t2audio->resume_work, t2audio_resume_work);
	INIT_LIST_HEAD(&t2audio->subdevice_list);
	t2audio->pcm_io_wq =
		alloc_ordered_workqueue("snd-apple-t2-io", WQ_MEM_RECLAIM);
	if (!t2audio->pcm_io_wq) {
		status = -ENOMEM;
		goto fail;
	}
	t2audio->msg_wq = alloc_workqueue("snd-apple-t2-msg", WQ_PERCPU, 0);
	if (!t2audio->msg_wq) {
		status = -ENOMEM;
		goto fail;
	}

	/* sets PCI_COMMAND_MEMORY and PCI_COMMAND_MASTER once more */
	if (pci_read_config_dword(dev, PCI_COMMAND, &cfg))
		dev_warn(&dev->dev, "pci_read_config_dword fail\n");
	if (pci_write_config_dword(dev, PCI_COMMAND,
				   cfg | PCI_COMMAND_MEMORY |
					   PCI_COMMAND_MASTER))
		dev_warn(&dev->dev, "pci_write_config_dword fail\n");

	t2audio->reg_mem_bs_dma = pci_resource_start(dev, 0);
	t2audio->bs_wc = pci_resource_flags(dev, 0) & IORESOURCE_PREFETCH;
	t2audio->reg_mem_bs = t2audio->bs_wc ? pci_iomap_wc(dev, 0, 0) :
					       pci_iomap(dev, 0, 0);
	t2audio->reg_mem_cfg = pci_iomap(dev, 4, 0);

	t2audio->reg_mem_gpr = t2audio->reg_mem_cfg + T2AUDIO_GPR_OFFSET;

	if (IS_ERR_OR_NULL(t2audio->reg_mem_bs) ||
	    IS_ERR_OR_NULL(t2audio->reg_mem_cfg)) {
		dev_err(&dev->dev, "failed to map BARs\n");
		goto fail;
	}

	if (t2audio_bce_init(t2audio)) {
		dev_err(&dev->dev, "failed to create the bridge queues\n");
		goto fail;
	}

	if (snd_card_new(t2audio->dev, t2audio_alsa_index, t2audio_alsa_id,
			 THIS_MODULE, 0, &t2audio->card)) {
		dev_err(&dev->dev, "failed to create the ALSA card\n");
		goto fail;
	}

	strscpy(t2audio->card->shortname, "Apple T2 Audio");
	strscpy(t2audio->card->longname, "Apple T2 Audio");
	strscpy(t2audio->card->mixername, "Apple T2 Audio");
	/* PCM devices without a fixed number in t2audio_alsa_id_mappings */
	t2audio->next_alsa_id = 100;

	if (t2audio_init_cmd(t2audio)) {
		dev_err(&dev->dev, "failed to enumerate the audio devices\n");
		goto fail_snd;
	}

	if (t2audio_init_bs(t2audio)) {
		dev_err(&dev->dev, "failed to map the BufferStruct\n");
		goto fail_snd;
	}

	status = t2audio_debugfs_init(t2audio);
	if (status) {
		dev_err(&dev->dev, "failed to create debugfs: %d\n", status);
		goto fail_snd;
	}

	status = t2audio_cmd_set_remote_access(t2audio,
					       T2AUDIO_REMOTE_ACCESS_ON);
	if (status) {
		dev_err(&dev->dev, "failed to enable remote access\n");
		goto fail_snd;
	}

	if (snd_card_register(t2audio->card)) {
		dev_err(&dev->dev, "failed to register the ALSA card\n");
		goto fail_snd;
	}

	list_for_each_entry(sdev, &t2audio->subdevice_list, list) {
		struct t2audio_buffer_struct_device *dev =
			&t2audio->bs->devices[sdev->buf_id];

		if (sdev->out_stream_cnt == 1 &&
		    !strcmp(dev->name, "Speaker")) {
			struct snd_pcm_hardware *hw =
				sdev->out_streams[0].alsa_hw_desc;

			/* the UCM profile selects the speaker setup by this */
			snprintf(t2audio->card->driver,
				 sizeof(t2audio->card->driver), "AppleT2x%d",
				 hw->channels_min);
		}
	}
	return 0;

fail_snd:
	t2audio_bce_exit(t2audio);
	debugfs_remove_recursive(t2audio->debugfs_dir);
	t2audio->debugfs_dir = NULL;
	snd_card_free(t2audio->card);
	while (!list_empty(&t2audio->subdevice_list)) {
		sdev = list_first_entry(&t2audio->subdevice_list,
					struct t2audio_subdevice, list);
		list_del(&sdev->list);
		t2audio_free_dev(sdev);
	}
fail:
	if (t2audio) {
		t2audio_bce_exit(t2audio);
		if (!IS_ERR_OR_NULL(t2audio->reg_mem_bs))
			pci_iounmap(dev, t2audio->reg_mem_bs);
		if (!IS_ERR_OR_NULL(t2audio->reg_mem_cfg))
			pci_iounmap(dev, t2audio->reg_mem_cfg);
		if (t2audio->pcm_io_wq)
			destroy_workqueue(t2audio->pcm_io_wq);
		kfree(t2audio);
	}

	pci_release_regions(dev);
	pci_disable_device(dev);

	if (!status)
		status = -EINVAL;
	return status;
}

static void t2audio_remove(struct pci_dev *dev)
{
	struct t2audio_subdevice *sdev;
	struct t2audio_device *t2audio = pci_get_drvdata(dev);

	cancel_work_sync(&t2audio->resume_work);
	list_for_each_entry(sdev, &t2audio->subdevice_list, list)
		t2audio_pcm_quiesce_subdevice(sdev);
	t2audio_bce_stop_messages(t2audio);
	debugfs_remove_recursive(t2audio->debugfs_dir);
	t2audio->debugfs_dir = NULL;
	snd_card_free(t2audio->card);
	while (!list_empty(&t2audio->subdevice_list)) {
		sdev = list_first_entry(&t2audio->subdevice_list,
					struct t2audio_subdevice, list);
		list_del(&sdev->list);
		t2audio_free_dev(sdev);
	}
	destroy_workqueue(t2audio->pcm_io_wq);
	t2audio_bce_exit(t2audio);
	pci_iounmap(dev, t2audio->reg_mem_bs);
	pci_iounmap(dev, t2audio->reg_mem_cfg);
	pci_free_irq_vectors(dev);
	pci_release_regions(dev);
	pci_disable_device(dev);
	kfree(t2audio);
}

static int t2audio_quiesce(struct t2audio_device *t2audio, bool suspend_pcm)
{
	struct t2audio_subdevice *sdev;
	int status;

	if (t2audio->pm_quiesced)
		return 0;

	cancel_work_sync(&t2audio->resume_work);
	t2audio->resume_deferred = false;

	/* stop PCM I/O while the bridge queues still work */
	list_for_each_entry(sdev, &t2audio->subdevice_list, list) {
		if (t2audio_pcm_quiesce_subdevice(sdev))
			dev_warn(t2audio->dev,
				 "Failed to stop %s during quiesce\n",
				 sdev->uid);

		if (suspend_pcm && sdev->pcm)
			snd_pcm_suspend_all(sdev->pcm);
	}
	flush_workqueue(t2audio->pcm_io_wq);

	status = t2audio_cmd_set_remote_access(t2audio,
					       T2AUDIO_REMOTE_ACCESS_OFF);
	if (status)
		dev_warn(t2audio->dev, "Failed to reset remote access\n");
	else
		t2audio->pm_quiesced = true;

	return status;
}

/*
 * Quiesce in ->prepare. The prepare phase completes for every device before
 * any ->suspend runs, so remote access is off before the USB devices behind
 * the T2 start their suspend traffic, with or without pm_async. The T2 stops
 * answering when both run concurrently.
 */
static int t2audio_prepare(struct device *dev)
{
	t2audio_quiesce(dev_get_drvdata(dev), true);
	return 0;
}

static int t2audio_suspend(struct device *dev)
{
	struct t2audio_device *t2audio = dev_get_drvdata(dev);

	iowrite32(T2AUDIO_SLEEP_MARKER,
		  &t2audio->reg_mem_gpr[T2AUDIO_GPR_SLEEP_MARKER / 4]);
	pci_disable_device(t2audio->pci);
	t2audio->pm_suspended = true;
	return 0;
}

static int t2audio_resume(struct device *dev)
{
	struct t2audio_device *t2audio = dev_get_drvdata(dev);
	int ret;

	ret = pci_enable_device(t2audio->pci);
	if (ret)
		return ret;
	pci_set_master(t2audio->pci);

	/*
	 * The buffers are registered again in ->complete, once the USB
	 * devices behind the T2 have resumed.
	 */
	t2audio->resume_deferred = true;
	return 0;
}

static void t2audio_complete(struct device *dev)
{
	struct t2audio_device *t2audio = dev_get_drvdata(dev);

	if (t2audio->pm_suspended) {
		t2audio->pm_suspended = false;
		if (t2audio->resume_deferred)
			schedule_work(&t2audio->resume_work);
		return;
	}

	/* an aborted suspend only ran ->prepare */
	if (!t2audio->pm_quiesced)
		return;

	if (t2audio_cmd_set_remote_access(t2audio, T2AUDIO_REMOTE_ACCESS_ON)) {
		dev_err(dev, "failed to enable remote access\n");
		return;
	}
	t2audio->pm_quiesced = false;
	t2audio_reset_streams(t2audio);
}

static void t2audio_shutdown(struct pci_dev *dev)
{
	struct t2audio_device *t2audio = pci_get_drvdata(dev);

	if (!t2audio)
		return;

	/* stop audio while the bridge queues still work */
	t2audio_quiesce(t2audio, false);
	pci_disable_device(t2audio->pci);
}

static void t2audio_resume_work(struct work_struct *ws)
{
	struct t2audio_device *t2audio =
		container_of(ws, struct t2audio_device, resume_work);

	if (!t2audio->resume_deferred)
		return;

	if (t2audio_reset_buffers(t2audio)) {
		t2audio->resume_deferred = false;
		dev_err(t2audio->dev, "failed to reset the buffers\n");
		return;
	}
	if (t2audio_cmd_set_remote_access(t2audio, T2AUDIO_REMOTE_ACCESS_ON)) {
		t2audio->resume_deferred = false;
		dev_err(t2audio->dev, "failed to enable remote access\n");
		return;
	}
	t2audio->resume_deferred = false;
	t2audio->pm_quiesced = false;
	t2audio_reset_streams(t2audio);
}

static void t2audio_reset_stream(struct t2audio_stream *stream)
{
	t2audio_pcm_quiesce_stream(stream);
	t2audio_pcm_reset_timing(stream);
}

static void t2audio_reset_streams(struct t2audio_device *a)
{
	struct t2audio_subdevice *sdev;
	size_t i;

	list_for_each_entry(sdev, &a->subdevice_list, list) {
		for (i = 0; i < sdev->in_stream_cnt; i++)
			t2audio_reset_stream(&sdev->in_streams[i]);
		for (i = 0; i < sdev->out_stream_cnt; i++)
			t2audio_reset_stream(&sdev->out_streams[i]);
	}
}

static int t2audio_init_cmd(struct t2audio_device *a)
{
	int status;
	struct t2audio_send_ctx sctx;
	struct t2audio_msg buf;
	u64 dev_cnt, dev_i;
	t2audio_device_id_t *dev_l;

	status = t2audio_send(a, &sctx, 500,
			      t2audio_msg_write_alive_notification, 1, 3);
	if (status) {
		dev_err(a->dev, "Sending alive notification failed\n");
		return status;
	}

	if (wait_for_completion_timeout(&a->remote_alive,
					msecs_to_jiffies(500)) == 0) {
		dev_err(a->dev, "Timed out waiting for remote\n");
		return -ETIMEDOUT;
	}
	pr_debug("Continuing init\n");

	buf = t2audio_reply_alloc();
	status = t2audio_cmd_get_device_list(a, &buf, &dev_l, &dev_cnt);
	if (status) {
		dev_err(a->dev, "Failed to get device list\n");
		t2audio_reply_free(&buf);
		return status;
	}
	for (dev_i = 0; dev_i < dev_cnt; ++dev_i)
		t2audio_init_dev(a, dev_l[dev_i]);
	t2audio_reply_free(&buf);

	return 0;
}

static void t2audio_init_stream_info(struct t2audio_subdevice *sdev,
				     struct t2audio_stream *strm);
static void
t2audio_handle_jack_connection_change(struct t2audio_subdevice *sdev);

static void t2audio_init_dev(struct t2audio_device *a,
			     t2audio_device_id_t dev_id)
{
	struct t2audio_subdevice *sdev;
	struct t2audio_msg buf = t2audio_reply_alloc();
	u64 uid_len, stream_cnt, i;
	t2audio_object_id_t *stream_list;
	char *uid;

	sdev = kzalloc_obj(*sdev);

	if (t2audio_cmd_get_property(a, &buf, dev_id, dev_id,
				     t2audio_prop(T2AUDIO_PROP_SCOPE_GLOBAL,
						  T2AUDIO_PROP_UID, 0),
				     NULL, 0, (void **)&uid, &uid_len) ||
	    uid_len > T2AUDIO_DEVICE_MAX_UID_LEN) {
		dev_err(a->dev, "Failed to get device uid for device %llx\n",
			dev_id);
		goto fail;
	}
	uid[uid_len] = '\0';
	pr_debug("Remote device %llx %.*s\n", dev_id, (int)uid_len, uid);

	sdev->a = a;
	INIT_LIST_HEAD(&sdev->list);
	sdev->dev_id = dev_id;
	sdev->buf_id = T2AUDIO_BUFFER_ID_NONE;
	memcpy(sdev->uid, uid, uid_len);
	sdev->uid[uid_len] = '\0';

	if (t2audio_cmd_get_primitive_property(
		    a, dev_id, dev_id,
		    t2audio_prop(T2AUDIO_PROP_SCOPE_INPUT, T2AUDIO_PROP_LATENCY,
				 0),
		    NULL, 0, &sdev->in_latency, sizeof(u32)))
		dev_warn(a->dev, "Failed to query device input latency\n");
	if (t2audio_cmd_get_primitive_property(
		    a, dev_id, dev_id,
		    t2audio_prop(T2AUDIO_PROP_SCOPE_OUTPUT,
				 T2AUDIO_PROP_LATENCY, 0),
		    NULL, 0, &sdev->out_latency, sizeof(u32)))
		dev_warn(a->dev, "Failed to query device output latency\n");
	if (t2audio_cmd_get_primitive_property(
		    a, dev_id, dev_id,
		    t2audio_prop(T2AUDIO_PROP_SCOPE_INPUT,
				 T2AUDIO_PROP_SAFETY_OFFSET, 0),
		    NULL, 0, &sdev->in_safety_offset, sizeof(u32)))
		dev_warn(a->dev,
			 "Failed to query device input safety offset\n");
	if (t2audio_cmd_get_primitive_property(
		    a, dev_id, dev_id,
		    t2audio_prop(T2AUDIO_PROP_SCOPE_OUTPUT,
				 T2AUDIO_PROP_SAFETY_OFFSET, 0),
		    NULL, 0, &sdev->out_safety_offset, sizeof(u32)))
		dev_warn(a->dev,
			 "Failed to query device output safety offset\n");
	dev_info(a->dev,
		 "%s: latency in=%u out=%u safety offset in=%u out=%u\n",
		 sdev->uid, sdev->in_latency, sdev->out_latency,
		 sdev->in_safety_offset, sdev->out_safety_offset);

	if (t2audio_cmd_get_input_stream_list(a, &buf, dev_id, &stream_list,
					      &stream_cnt)) {
		dev_err(a->dev,
			"Failed to get input stream list for device %llx\n",
			dev_id);
		goto fail;
	}
	if (stream_cnt > T2AUDIO_DEVICE_MAX_INPUT_STREAMS) {
		dev_warn(
			a->dev,
			"Device %s input stream count %llu is larger than the supported count of %u\n",
			sdev->uid, stream_cnt,
			T2AUDIO_DEVICE_MAX_INPUT_STREAMS);
		stream_cnt = T2AUDIO_DEVICE_MAX_INPUT_STREAMS;
	}
	/*
	 * Only Digital Mic and Codec Input take a host input buffer. The T2
	 * fails to map one for Bridge Loopback, it is larger than the T2 side
	 * loopback buffer.
	 */
	if (!strcmp(sdev->uid, "Bridge Loopback"))
		stream_cnt = 0;
	sdev->in_stream_cnt = stream_cnt;
	for (i = 0; i < stream_cnt; i++) {
		sdev->in_streams[i].id = stream_list[i];
		sdev->in_streams[i].buffer_cnt = 0;
		t2audio_init_stream_info(sdev, &sdev->in_streams[i]);
		sdev->in_streams[i].latency += sdev->in_latency;
	}

	if (t2audio_cmd_get_output_stream_list(a, &buf, dev_id, &stream_list,
					       &stream_cnt)) {
		dev_err(a->dev,
			"Failed to get output stream list for device %llx\n",
			dev_id);
		goto fail;
	}
	if (stream_cnt > T2AUDIO_DEVICE_MAX_OUTPUT_STREAMS) {
		dev_warn(
			a->dev,
			"Device %s output stream count %llu is larger than the supported count of %u\n",
			sdev->uid, stream_cnt,
			T2AUDIO_DEVICE_MAX_OUTPUT_STREAMS);
		stream_cnt = T2AUDIO_DEVICE_MAX_OUTPUT_STREAMS;
	}
	sdev->out_stream_cnt = stream_cnt;
	for (i = 0; i < stream_cnt; i++) {
		sdev->out_streams[i].id = stream_list[i];
		sdev->out_streams[i].buffer_cnt = 0;
		t2audio_init_stream_info(sdev, &sdev->out_streams[i]);
		sdev->out_streams[i].latency += sdev->out_latency;
	}

	if (sdev->is_pcm)
		t2audio_create_pcm(sdev);
	/*
	 * The iMacPro1,1 reports one "Codec" device instead of "Codec Output"
	 * and "Codec Input". Its jack keeps the "Codec Output" name so the UCM
	 * JackControl matches on every model.
	 */
	if (!strcmp(sdev->uid, "Codec Output") || !strcmp(sdev->uid, "Codec")) {
		if (snd_jack_new(a->card, "Codec Output", SND_JACK_HEADPHONE,
				 &sdev->jack, true, false))
			dev_warn(a->dev,
				 "Failed to create an attached jack for %s\n",
				 sdev->uid);
		t2audio_cmd_property_listener(
			a, sdev->dev_id, sdev->dev_id,
			t2audio_prop(T2AUDIO_PROP_SCOPE_OUTPUT,
				     T2AUDIO_PROP_JACK_PLUGGED, 0));
		t2audio_handle_jack_connection_change(sdev);
	}

	t2audio_reply_free(&buf);

	list_add_tail(&sdev->list, &a->subdevice_list);
	return;

fail:
	t2audio_reply_free(&buf);
	kfree(sdev);
}

static void t2audio_init_stream_info(struct t2audio_subdevice *sdev,
				     struct t2audio_stream *strm)
{
	if (t2audio_cmd_get_primitive_property(
		    sdev->a, sdev->dev_id, strm->id,
		    t2audio_prop(T2AUDIO_PROP_SCOPE_GLOBAL,
				 T2AUDIO_PROP_PHYS_FORMAT, 0),
		    NULL, 0, &strm->desc, sizeof(strm->desc)))
		dev_warn(sdev->a->dev, "Failed to query stream descriptor\n");
	if (t2audio_cmd_get_primitive_property(
		    sdev->a, sdev->dev_id, strm->id,
		    t2audio_prop(T2AUDIO_PROP_SCOPE_GLOBAL,
				 T2AUDIO_PROP_LATENCY, 0),
		    NULL, 0, &strm->latency, sizeof(u32)))
		dev_warn(sdev->a->dev, "Failed to query stream latency\n");
	if (strm->desc.format_id == T2AUDIO_FORMAT_LPCM)
		sdev->is_pcm = true;
}

static void t2audio_free_dev(struct t2audio_subdevice *sdev)
{
	size_t i;

	t2audio_pcm_cleanup_subdevice(sdev);
	for (i = 0; i < sdev->in_stream_cnt; i++) {
		struct t2audio_dma_buf *buf = sdev->in_streams[i].buffers;

		kfree(sdev->in_streams[i].alsa_hw_desc);
		if (buf) {
			size_t j;

			for (j = 0; j < sdev->in_streams[i].buffer_cnt; j++) {
				if (buf[j].type == T2AUDIO_DMA_BUF_COHERENT)
					dma_free_coherent(&sdev->a->pci->dev,
							  buf[j].size,
							  buf[j].ptr,
							  buf[j].dma_addr);
			}
			kfree(buf);
		}
	}
	for (i = 0; i < sdev->out_stream_cnt; i++) {
		kfree(sdev->out_streams[i].alsa_hw_desc);
		kfree(sdev->out_streams[i].buffers);
	}
	kfree(sdev);
}

static struct t2audio_subdevice *
t2audio_find_dev_by_dev_id(struct t2audio_device *a, t2audio_device_id_t dev_id)
{
	struct t2audio_subdevice *sdev;

	list_for_each_entry(sdev, &a->subdevice_list, list) {
		if (dev_id == sdev->dev_id)
			return sdev;
	}
	return NULL;
}

static struct t2audio_subdevice *
t2audio_find_dev_by_uid(struct t2audio_device *a, const char *uid)
{
	struct t2audio_subdevice *sdev;

	list_for_each_entry(sdev, &a->subdevice_list, list) {
		if (!strcmp(uid, sdev->uid))
			return sdev;
	}
	return NULL;
}

static void
t2audio_init_bs_stream(struct t2audio_device *a, struct t2audio_stream *strm,
		       struct t2audio_buffer_struct_stream *bs_strm);
static void
t2audio_remap_bs_stream(struct t2audio_device *a, struct t2audio_stream *strm,
			struct t2audio_buffer_struct_stream *bs_strm);
static void t2audio_init_bs_stream_host(struct t2audio_device *a,
					struct t2audio_stream *strm);

static int t2audio_map_bs(struct t2audio_device *a, bool remap)
{
	int i, j;
	struct t2audio_buffer_struct_device *dev;
	struct t2audio_subdevice *sdev;
	u32 ver, sig, bs_base;

	ver = ioread32(&a->reg_mem_gpr[0]);
	if (ver < 3) {
		dev_err(a->dev, "Bad GPR version (%u)", ver);
		return -EINVAL;
	}
	sig = ioread32(&a->reg_mem_gpr[1]);
	if (sig != T2AUDIO_SIG) {
		dev_err(a->dev, "Bad GPR sig (%x)", sig);
		return -EINVAL;
	}
	bs_base = ioread32(&a->reg_mem_gpr[2]);
	a->bs = (struct t2audio_buffer_struct *)((u8 *)a->reg_mem_bs + bs_base);
	if (a->bs->signature != T2AUDIO_SIG) {
		dev_err(a->dev, "Bad BufferStruct sig (%x)", a->bs->signature);
		return -EINVAL;
	}
	pr_debug("BufferStruct ver = %i\n", a->bs->version);
	pr_debug("Num devices = %i\n", a->bs->num_devices);
	if (remap) {
		list_for_each_entry(sdev, &a->subdevice_list, list)
			sdev->buf_id = T2AUDIO_BUFFER_ID_NONE;
	}
	for (i = 0; i < a->bs->num_devices; i++) {
		dev = &a->bs->devices[i];
		pr_debug("Device %i %s\n", i, dev->name);

		sdev = t2audio_find_dev_by_uid(a, dev->name);
		if (!sdev) {
			dev_err(a->dev,
				"Subdevice not found for BufferStruct device %s\n",
				dev->name);
			continue;
		}
		sdev->buf_id = (u8)i;
		for (j = 0; j < dev->num_output_streams; j++) {
			pr_debug(
				"Device %i Stream %i: Output; Buffer Count = %i\n",
				i, j, dev->output_streams[j].num_buffers);
			if (j >= sdev->out_stream_cnt)
				continue;
			if (remap)
				t2audio_remap_bs_stream(
					a, &sdev->out_streams[j],
					&dev->output_streams[j]);
			else
				t2audio_init_bs_stream(a, &sdev->out_streams[j],
						       &dev->output_streams[j]);
		}
	}

	list_for_each_entry(sdev, &a->subdevice_list, list) {
		if (sdev->buf_id != T2AUDIO_BUFFER_ID_NONE)
			continue;
		sdev->buf_id = i;
		pr_debug("Created device %i %s\n", i, sdev->uid);
		strscpy(a->bs->devices[i].name, sdev->uid);
		a->bs->devices[i].num_input_streams = 0;
		a->bs->devices[i].num_output_streams = 0;
		a->bs->num_devices = ++i;
	}

	return 0;
}

static void t2audio_reset_input_streams(struct t2audio_device *a)
{
	struct t2audio_buffer_struct_device *dev;
	int i;

	for (i = 0; i < a->bs->num_devices && i < ARRAY_SIZE(a->bs->devices);
	     i++) {
		dev = &a->bs->devices[i];
		dev->input_streams[0].buffers[0].address = 0;
		dev->input_streams[0].buffers[0].size = 0;
		dev->input_streams[0].num_buffers = 0;
		dev->num_input_streams = 0;
	}
	/* the T2 reads the BufferStruct, publish the entries */
	wmb();
}

static void t2audio_register_input_stream(struct t2audio_device *a,
					  struct t2audio_subdevice *sdev)
{
	struct t2audio_buffer_struct_device *dev =
		&a->bs->devices[sdev->buf_id];
	struct t2audio_stream *strm = &sdev->in_streams[0];

	if (!strm->buffer_cnt)
		return;
	pr_debug("Device %i Host Stream; Input size %zx\n", sdev->buf_id,
		 strm->buffers[0].size);
	dev->input_streams[0].buffers[0].address = strm->buffers[0].dma_addr;
	dev->input_streams[0].buffers[0].size = strm->buffers[0].size;
	dev->input_streams[0].num_buffers = 1;
	dev->num_input_streams = 1;
	/* the entry must be visible before the T2 is told about it */
	wmb();

	if (t2audio_cmd_set_input_stream_address_ranges(a, sdev->dev_id))
		dev_err(a->dev, "Failed to set input stream address ranges\n");
}

static void t2audio_register_input_streams(struct t2audio_device *a)
{
	struct t2audio_subdevice *sdev;

	list_for_each_entry(sdev, &a->subdevice_list, list) {
		if (sdev->in_stream_cnt == 1)
			t2audio_register_input_stream(a, sdev);
	}
}

static int t2audio_init_bs(struct t2audio_device *a)
{
	struct t2audio_subdevice *sdev;
	int status;

	status = t2audio_map_bs(a, false);
	if (status)
		return status;
	list_for_each_entry(sdev, &a->subdevice_list, list) {
		if (sdev->in_stream_cnt == 1)
			t2audio_init_bs_stream_host(a, &sdev->in_streams[0]);
	}
	t2audio_reset_input_streams(a);
	t2audio_register_input_streams(a);
	return 0;
}

/*
 * A missing sleep marker means the T2 lost its state and the BufferStruct
 * has to be mapped again. Either way the host input buffers are registered
 * again before remote access returns.
 */
static int t2audio_reset_buffers(struct t2audio_device *a)
{
	int status;

	if (ioread32(&a->reg_mem_gpr[T2AUDIO_GPR_SLEEP_MARKER / 4]) !=
	    T2AUDIO_SLEEP_MARKER) {
		dev_dbg(a->dev,
			"T2 lost its state, remapping the BufferStruct\n");
		status = t2audio_map_bs(a, true);
		if (status)
			return status;
	}
	t2audio_reset_input_streams(a);
	t2audio_register_input_streams(a);
	return 0;
}

static ssize_t t2audio_debugfs_buffer_struct_read(struct file *file,
						  char __user *user_buf,
						  size_t count, loff_t *ppos)
{
	struct t2audio_device *a = file->private_data;
	const size_t total = sizeof(*a->bs);
	size_t done = 0;
	u8 *buf;

	if (*ppos < 0)
		return -EINVAL;
	if ((size_t)*ppos >= total || !count)
		return 0;

	count = min_t(size_t, count, total - (size_t)*ppos);
	buf = kmalloc(min_t(size_t, count, PAGE_SIZE), GFP_KERNEL);
	if (!buf)
		return -ENOMEM;

	while (done < count) {
		size_t chunk = min_t(size_t, count - done, PAGE_SIZE);

		memcpy_fromio(buf, (u8 __iomem *)a->bs + (size_t)*ppos + done,
			      chunk);
		if (copy_to_user(user_buf + done, buf, chunk)) {
			kfree(buf);
			return done ? (ssize_t)done : -EFAULT;
		}
		done += chunk;
	}

	kfree(buf);
	*ppos += done;
	return done;
}

static const struct file_operations t2audio_debugfs_buffer_struct_fops = {
	.owner = THIS_MODULE,
	.open = simple_open,
	.read = t2audio_debugfs_buffer_struct_read,
	.llseek = default_llseek,
};

static int t2audio_debugfs_init(struct t2audio_device *a)
{
	struct dentry *file;
	int status;

	a->debugfs_dir = debugfs_create_dir(KBUILD_MODNAME, NULL);
	if (IS_ERR_OR_NULL(a->debugfs_dir)) {
		status = a->debugfs_dir ? PTR_ERR(a->debugfs_dir) : -ENOMEM;
		a->debugfs_dir = NULL;
		return status;
	}

	file = debugfs_create_file("buffer_struct", 0400, a->debugfs_dir, a,
				   &t2audio_debugfs_buffer_struct_fops);
	if (IS_ERR_OR_NULL(file)) {
		status = file ? PTR_ERR(file) : -ENOMEM;
		debugfs_remove_recursive(a->debugfs_dir);
		a->debugfs_dir = NULL;
		return status;
	}

	return 0;
}

static void t2audio_init_bs_stream(struct t2audio_device *a,
				   struct t2audio_stream *strm,
				   struct t2audio_buffer_struct_stream *bs_strm)
{
	size_t i;

	strm->buffer_cnt = bs_strm->num_buffers;
	if (bs_strm->num_buffers > T2AUDIO_DEVICE_MAX_BUFFER_COUNT) {
		dev_warn(
			a->dev,
			"BufferStruct buffer count %u exceeds driver limit of %u\n",
			bs_strm->num_buffers, T2AUDIO_DEVICE_MAX_BUFFER_COUNT);
		strm->buffer_cnt = T2AUDIO_DEVICE_MAX_BUFFER_COUNT;
	}
	if (!strm->buffer_cnt)
		return;
	strm->buffers = kmalloc_array(
		strm->buffer_cnt, sizeof(struct t2audio_dma_buf), GFP_KERNEL);
	if (!strm->buffers)
		return;
	for (i = 0; i < strm->buffer_cnt; i++) {
		strm->buffers[i].dma_addr =
			a->reg_mem_bs_dma +
			(dma_addr_t)bs_strm->buffers[i].address;
		strm->buffers[i].ptr =
			a->reg_mem_bs + bs_strm->buffers[i].address;
		strm->buffers[i].size = bs_strm->buffers[i].size;
		strm->buffers[i].type = T2AUDIO_DMA_BUF_IOMEM;
	}

	if (strm->buffer_cnt == 1) {
		strm->alsa_hw_desc = kmalloc_obj(*strm->alsa_hw_desc);
		if (t2audio_create_hw_info(&strm->desc, strm->alsa_hw_desc,
					   strm->buffers[0].size)) {
			kfree(strm->alsa_hw_desc);
			strm->alsa_hw_desc = NULL;
		}
	}
}

/* the T2 lost its state: point the output buffers at the new BufferStruct */
static void
t2audio_remap_bs_stream(struct t2audio_device *a, struct t2audio_stream *strm,
			struct t2audio_buffer_struct_stream *bs_strm)
{
	size_t i;

	if (!strm->buffers) {
		t2audio_init_bs_stream(a, strm, bs_strm);
		return;
	}
	if (min_t(size_t, bs_strm->num_buffers,
		  T2AUDIO_DEVICE_MAX_BUFFER_COUNT) != strm->buffer_cnt) {
		dev_err(a->dev, "BufferStruct buffer count changed to %u\n",
			bs_strm->num_buffers);
		return;
	}
	for (i = 0; i < strm->buffer_cnt; i++) {
		if (bs_strm->buffers[i].size != strm->buffers[i].size) {
			dev_err(a->dev,
				"BufferStruct buffer size changed from %zx to %zx\n",
				strm->buffers[i].size,
				bs_strm->buffers[i].size);
			continue;
		}
		strm->buffers[i].dma_addr =
			a->reg_mem_bs_dma +
			(dma_addr_t)bs_strm->buffers[i].address;
		strm->buffers[i].ptr =
			a->reg_mem_bs + bs_strm->buffers[i].address;
	}
}

static void t2audio_init_bs_stream_host(struct t2audio_device *a,
					struct t2audio_stream *strm)
{
	size_t size;
	dma_addr_t dma_addr;
	void *dma_ptr;

	size = strm->desc.bytes_per_packet * 16640;
	dma_ptr = dma_alloc_coherent(&a->pci->dev, size, &dma_addr, GFP_KERNEL);
	if (!dma_ptr) {
		dev_err(a->dev, "dma_alloc_coherent failed\n");
		return;
	}

	memset(dma_ptr, 0, size);

	strm->buffer_cnt = 1;
	strm->buffers = kmalloc_array(
		strm->buffer_cnt, sizeof(struct t2audio_dma_buf), GFP_KERNEL);
	if (!strm->buffers) {
		dma_free_coherent(&a->pci->dev, size, dma_ptr, dma_addr);
		strm->buffer_cnt = 0;
		return;
	}
	strm->buffers[0].dma_addr = dma_addr;
	strm->buffers[0].ptr = dma_ptr;
	strm->buffers[0].size = size;
	strm->buffers[0].type = T2AUDIO_DMA_BUF_COHERENT;

	strm->alsa_hw_desc = kmalloc_obj(*strm->alsa_hw_desc);
	if (t2audio_create_hw_info(&strm->desc, strm->alsa_hw_desc,
				   strm->buffers[0].size)) {
		kfree(strm->alsa_hw_desc);
		strm->alsa_hw_desc = NULL;
	}
}

static void t2audio_handle_prop_change(struct t2audio_device *a,
				       struct t2audio_msg *msg);

void t2audio_handle_notification(struct t2audio_device *a,
				 struct t2audio_msg *msg)
{
	struct t2audio_send_ctx sctx;
	struct t2audio_msg_base base;

	if (t2audio_msg_read_base(msg, &base))
		return;
	switch (base.msg) {
	case T2AUDIO_MSG_NOTIFICATION_BOOT:
		pr_debug("Received boot notification from remote\n");

		if (t2audio_send(a, &sctx, 500,
				 t2audio_msg_write_alive_notification, 1, 3)) {
			pr_err("Sending alive notification failed\n");
		}
		break;
	case T2AUDIO_MSG_NOTIFICATION_ALIVE:
		pr_debug("Received alive notification from remote\n");
		complete_all(&a->remote_alive);
		break;
	case T2AUDIO_MSG_PROPERTY_CHANGED:
		t2audio_handle_prop_change(a, msg);
		break;
	default:
		pr_debug("Unhandled notification %i\n", base.msg);
		break;
	}
}

struct t2audio_prop_change_work_struct {
	struct work_struct ws;
	struct t2audio_device *a;
	t2audio_device_id_t dev;
	t2audio_object_id_t obj;
	struct t2audio_prop_addr prop;
};

static void
t2audio_handle_jack_connection_change(struct t2audio_subdevice *sdev)
{
	u32 plugged;

	if (!sdev->jack)
		return;
	/* the T2 reports the jack per scope, the output scope covers both */
	if (t2audio_cmd_get_primitive_property(
		    sdev->a, sdev->dev_id, sdev->dev_id,
		    t2audio_prop(T2AUDIO_PROP_SCOPE_OUTPUT,
				 T2AUDIO_PROP_JACK_PLUGGED, 0),
		    NULL, 0, &plugged, sizeof(plugged))) {
		dev_err(sdev->a->dev, "Failed to get jack enable status\n");
		return;
	}
	pr_debug("Jack is now %s\n", plugged ? "plugged" : "unplugged");
	snd_jack_report(sdev->jack, plugged ? sdev->jack->type : 0);
}

void t2audio_handle_prop_change_work(struct work_struct *ws)
{
	struct t2audio_prop_change_work_struct *work =
		container_of(ws, struct t2audio_prop_change_work_struct, ws);
	struct t2audio_subdevice *sdev;

	sdev = t2audio_find_dev_by_dev_id(work->a, work->dev);
	if (!sdev) {
		dev_err(work->a->dev,
			"Property notification change: device not found\n");
		goto done;
	}
	pr_debug("Property changed for device: %s\n", sdev->uid);

	if (work->prop.scope == T2AUDIO_PROP_SCOPE_OUTPUT &&
	    work->prop.selector == T2AUDIO_PROP_JACK_PLUGGED) {
		t2audio_handle_jack_connection_change(sdev);
	}

done:
	kfree(work);
}

void t2audio_handle_prop_change(struct t2audio_device *a,
				struct t2audio_msg *msg)
{
	/* querying the device waits for replies, so leave the receive path */
	struct t2audio_prop_change_work_struct *work;

	work = kmalloc_obj(*work);
	work->a = a;
	INIT_WORK(&work->ws, t2audio_handle_prop_change_work);
	t2audio_msg_read_property_changed(msg, &work->dev, &work->obj,
					  &work->prop);
	queue_work(a->msg_wq, &work->ws);
}

#define t2audio_send_cmd_response(a, sctx, msg, fn, ...)                     \
	do {                                                                 \
		if (t2audio_send_with_tag(                                   \
			    a, sctx,                                         \
			    ((struct t2audio_msg_header *)(msg)->data)->tag, \
			    500, fn, ##__VA_ARGS__))                         \
			pr_err("Failed to reply to a command\n");            \
	} while (0)

void t2audio_handle_cmd_timestamp(struct t2audio_device *a,
				  struct t2audio_msg *msg)
{
	struct t2audio_send_ctx sctx;
	struct t2audio_subdevice *sdev;
	u64 devid, timestamp, update_seed, sample_time, sample_frames;

	if (t2audio_msg_read_update_timestamp(msg, &devid, &timestamp,
					      &update_seed, &sample_time) ||
	    !t2audio_decode_sample_time(sample_time, &sample_frames) ||
	    update_seed > 1) {
		dev_warn_ratelimited(
			a->dev,
			"Invalid or unsupported timestamp message (%zu bytes)\n",
			msg->size);
		goto reply;
	}

	pr_debug("timestamp dev=%llx t2=%llu host=%llu seed=%llu sample=%llu\n",
		 devid, timestamp, msg->received_ns, update_seed,
		 sample_frames);
	sdev = t2audio_find_dev_by_dev_id(a, devid);
	if (sdev)
		t2audio_handle_timestamp(sdev, msg->received_ns, timestamp,
					 update_seed, sample_frames);
reply:
	t2audio_send_cmd_response(a, &sctx, msg,
				  t2audio_msg_write_update_timestamp_response);
}

void t2audio_handle_command(struct t2audio_device *a, struct t2audio_msg *msg)
{
	struct t2audio_msg_base base;

	if (t2audio_msg_read_base(msg, &base))
		return;
	switch (base.msg) {
	case T2AUDIO_MSG_UPDATE_TIMESTAMP:
		t2audio_handle_cmd_timestamp(a, msg);
		break;
	default:
		pr_debug("Unhandled device command %i\n", base.msg);
		break;
	}
}

static const struct pci_device_id t2audio_ids[] = { { PCI_VDEVICE(APPLE,
								  0x1803) },
						    {} };
MODULE_DEVICE_TABLE(pci, t2audio_ids);

static const struct dev_pm_ops t2audio_pm_ops = {
	.prepare = t2audio_prepare,
	.suspend = t2audio_suspend,
	.resume = t2audio_resume,
	.complete = t2audio_complete,
};

static struct pci_driver t2audio_pci_driver = {
	.name = KBUILD_MODNAME,
	.id_table = t2audio_ids,
	.probe = t2audio_probe,
	.remove = t2audio_remove,
	.shutdown = t2audio_shutdown,
	.driver.pm = pm_sleep_ptr(&t2audio_pm_ops),
};
module_pci_driver(t2audio_pci_driver);

struct t2audio_alsa_pcm_id_mapping t2audio_alsa_id_mappings[] = {
	{ "Speaker", 0 },
	{ "Digital Mic", 1 },
	{ "Codec Output", 2 },
	{ "Codec", 2 },
	{ "Codec Input", 3 },
	{ "Bridge Loopback", 4 },
	{}
};

module_param_named(index, t2audio_alsa_index, int, 0444);
MODULE_PARM_DESC(index, "Index value for Apple Internal Audio soundcard.");
module_param_named(id, t2audio_alsa_id, charp, 0444);
MODULE_PARM_DESC(id, "ID string for Apple Internal Audio soundcard.");
MODULE_IMPORT_NS("APPLE_T2_BRIDGE");
MODULE_AUTHOR("Paul Pawlowski <paul@mrarm.io>");
MODULE_AUTHOR("André Eikmeyer <andre.eikmeyer@kait2en.org>");
MODULE_DESCRIPTION("Apple T2 audio");
MODULE_LICENSE("GPL");
