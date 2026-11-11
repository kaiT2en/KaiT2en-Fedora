// SPDX-License-Identifier: GPL-2.0
/*
 * Apple T2 video encoder (AVE), client of the Apple T2 Bridge Controller.
 *
 * Copyright (C) André Eikmeyer <andre.eikmeyer@kait2en.org>
 */

#define pr_fmt(fmt) KBUILD_MODNAME ": " fmt

#include <linux/apple-t2-bridge.h>
#include <linux/auxiliary_bus.h>
#include <linux/module.h>

#include "video.h"

static int t2_ave_probe(struct auxiliary_device *adev,
			   const struct auxiliary_device_id *id)
{
	struct apple_bce *bce = dev_get_platdata(&adev->dev);
	struct t2_ave_device *video;
	int ret;

	if (!bce)
		return -ENODEV;

	ret = t2_ave_video_create(bce, &video);
	if (ret)
		return ret;
	auxiliary_set_drvdata(adev, video);
	return 0;
}

static void t2_ave_remove(struct auxiliary_device *adev)
{
	t2_ave_video_destroy(auxiliary_get_drvdata(adev));
}

static void t2_ave_shutdown(struct auxiliary_device *adev)
{
	t2_ave_video_suspend(auxiliary_get_drvdata(adev));
}

static int t2_ave_suspend(struct device *dev)
{
	t2_ave_video_suspend(dev_get_drvdata(dev));
	return 0;
}

static const struct dev_pm_ops t2_ave_pm_ops = {
	.suspend = t2_ave_suspend,
};

static const struct auxiliary_device_id t2_ave_ids[] = {
	{ .name = "apple_t2_bridge.ave" },
	{ }
};
MODULE_DEVICE_TABLE(auxiliary, t2_ave_ids);

static struct auxiliary_driver t2_ave_driver = {
	.name = "ave",
	.id_table = t2_ave_ids,
	.probe = t2_ave_probe,
	.remove = t2_ave_remove,
	.shutdown = t2_ave_shutdown,
	.driver.pm = pm_sleep_ptr(&t2_ave_pm_ops),
};
module_auxiliary_driver(t2_ave_driver);

MODULE_IMPORT_NS("APPLE_T2_BRIDGE");
MODULE_LICENSE("GPL");
MODULE_AUTHOR("André Eikmeyer <andre.eikmeyer@kait2en.org>");
MODULE_DESCRIPTION("Apple T2 AVE");
