# Hardware video decoding

Fedora's GPU drivers do not expose hardware decoding for codecs such as H.264
and H.265. This affects both AMD GPUs and Intel iGPUs. Browsers and media
players can therefore fall back to CPU decoding even though the GPU contains a
supported video decoder.

T2 Macs do not provide AV1 hardware decoding. YouTube should use H.264 or a
hardware-supported VP9 profile to avoid unnecessary CPU load.

This page is about decoding on the GPUs. HEVC encoding runs on the T2 itself,
see [Hardware video encoding](hardware-video-encoding.md).

## Install the VA-API drivers

Run the installer from the KAIT2EN repository:

```bash
cd /usr/local/src/KaiT2en-Fedora
sudo bash ./scripts/fedora/install-hardware-video-decoding.sh
```

The script enables RPM Fusion, installs the appropriate VA-API drivers for every
detected Intel and AMD GPU, and checks each render node. Review the final output:
H.264 and VP9 decoding and encoding are reported as `available` or `missing` for
each GPU.

No general system upgrade is performed. Browser configuration is still required
as described below.

## Brave and YouTube

Open Brave's system settings and enable **Use graphics acceleration when
available**, then restart Brave.

YouTube normally chooses between H.264, VP9 and AV1 based on the capabilities
reported by the browser. T2 Macs cannot decode AV1 in hardware. To force
YouTube to request H.264 in Brave, install
[h264ify from the Chrome Web Store](https://chromewebstore.google.com/detail/h264ify/aleakchihdccplidncghkekgioiakgal).
This does not interfere with Brave Shields or its ad blocking.

H.264 streams on YouTube are commonly limited to 1080p. Disable the extension
when a higher resolution is more important than lower CPU use.

Start a YouTube video, right-click it and open **Stats for nerds**. The codec
field identifies the selected stream:

- `avc1` is H.264.
- `vp09` is VP9.
- `av01` is AV1.

For the forced H.264 configuration, the field must start with `avc1`.

Open `brave://gpu` and check that **Video Decode** reports hardware
acceleration. For the active playback session, `brave://media-internals`
provides the more useful per-stream decoder details.

## Firefox and YouTube

In Firefox settings, leave hardware acceleration enabled. Open `about:config`,
set the following preference to `false`, and restart Firefox:

```text
media.av1.enabled
```

This disables AV1 but still allows YouTube to select VP9. To force H.264,
install
[enhanced-h264ify from Mozilla Add-ons](https://addons.mozilla.org/firefox/addon/enhanced-h264ify/)
and block VP9 and AV1 while leaving H.264 enabled.

Use YouTube's **Stats for nerds** as described above. Firefox also reports its
decoder support under `about:support` in the codec and media sections.

## Test outside the browser

MPV can confirm that VA-API works independently of browser configuration:

```bash
mpv --no-config \
    --hwdec=vaapi-copy \
    --hwdec-codecs=all \
    --vaapi-device=/dev/dri/renderD128 \
    --term-playing-msg='Hardware decoder: ${hwdec-current}' \
    /path/to/video.mp4
```

Replace the render node and video path as required. A hardware-decoded stream
reports `vaapi-copy` instead of `no`.
