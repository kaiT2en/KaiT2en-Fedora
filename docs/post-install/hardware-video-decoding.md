# Hardware video decoding

Fedora's GPU drivers do not expose hardware decoding for codecs such as H.264
and H.265. This affects both AMD GPUs and Intel iGPUs. Browsers and media
players can therefore fall back to CPU decoding even though the GPU contains a
supported video decoder.

T2 Macs do not provide AV1 hardware decoding. YouTube should use H.264 or a
hardware-supported VP9 profile to avoid unnecessary CPU load.

## Install the VA-API drivers

KaiT2en can enable RPM Fusion, install the matching drivers for every detected
Intel or AMD GPU, and report the VA-API capabilities of each render node:

```bash
sudo bash ./scripts/fedora/install-hardware-video-decoding.sh
```

The script does not run a general system upgrade. Review its output after the
installation: H.264 and VP9 support is reported separately for every render
node. Browser configuration is still required as described below.

The same packages can be installed manually. If RPM Fusion is not enabled yet,
add its Free and Nonfree repositories:

```bash
sudo dnf install \
  "https://mirrors.rpmfusion.org/free/fedora/rpmfusion-free-release-$(rpm -E %fedora).noarch.rpm" \
  "https://mirrors.rpmfusion.org/nonfree/fedora/rpmfusion-nonfree-release-$(rpm -E %fedora).noarch.rpm"
```

Refresh package metadata and install the diagnostic utility:

```bash
sudo dnf makecache --refresh
sudo dnf install libva-utils
```

These commands deliberately do not run a general system upgrade. Review the DNF
transaction before accepting it, especially on systems using an out-of-tree
AMDGPU module.

For an AMD GPU, install the Freeworld Mesa VA-API driver:

```bash
sudo dnf install mesa-va-drivers-freeworld
```

For a modern Intel iGPU, replace Fedora's restricted iHD driver with the full
Intel media driver:

```bash
sudo dnf swap libva-intel-media-driver intel-media-driver
```

Install both drivers on a hybrid Intel/AMD system.

## Verify the decoder

DRM card and render-node numbers can change between boots. Locate each render
node by its PCI path:

```bash
for dev in /dev/dri/renderD*; do
    echo "=== $dev ==="
    udevadm info -q property -n "$dev" |
        grep -E 'DEVNAME|ID_PATH'
done
```

Then inspect the required node. Replace `renderD128` with the node found above:

```bash
vainfo --display drm --device /dev/dri/renderD128 |
    grep -E 'Driver version|H264|HEVC|VP9|MPEG2'
```

The output must list the codec that the browser should decode. On a hybrid
system, test both render nodes.

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
