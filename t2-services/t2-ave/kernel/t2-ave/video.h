#ifndef T2_AVE_VIDEO_H
#define T2_AVE_VIDEO_H

struct apple_bce;
struct t2_ave_device;

int t2_ave_video_create(struct apple_bce *client,
			   struct t2_ave_device **result);
void t2_ave_video_suspend(struct t2_ave_device *adev);
void t2_ave_video_destroy(struct t2_ave_device *adev);

#endif
