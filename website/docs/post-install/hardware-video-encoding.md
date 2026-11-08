# Hardware video encoding

The T2 contains Apple's video encoder, the AVE. KAIT2EN exposes it to Linux
twice, as two standard V4L2 memory-to-memory encoders, so ffmpeg, HandBrake and
anything else that speaks V4L2 can hand it raw frames and receive compressed
video. The Intel and AMD GPUs keep doing the decoding. The T2 only encodes.

| Device name | Path on the T2 | Output | Made for |
| --- | --- | --- | --- |
| `apple-ave-enc` | AVE bridge, like macOS VideoToolbox | HEVC, 8 and 10 bit | Transcoding files |
| `t2-sidecar` | The low-latency encoder macOS uses for Sidecar | H.264 and HEVC, 8 bit | Streaming the desktop, see [Sidecar](sidecar.md) |

`v4l2-ctl --list-devices` shows which `/dev/video` node each one got.

## AVE encoder

- HEVC (H.265) only. There is no H.264 output.
- Main profile from 8-bit `NV12` input, Main 10 from `P010` input.
- Frame sizes from 128x128 up to 4096x2304.
- One encode at a time. A second session is refused with `EBUSY` until the
  first has finished.

### ffmpeg

Fedora's `ffmpeg-free` already ships the V4L2 encoder wrapper. The encoder
expects `NV12` frames, so tell ffmpeg to convert:

```bash
ffmpeg -i input.mp4 -pix_fmt nv12 -c:v hevc_v4l2m2m -b:v 8M -c:a copy output.mp4
```

`-b:v` sets the target bitrate and `-g` the keyframe interval. `-profile:v`
and `-level` are passed through as well. Without `-pix_fmt nv12` ffmpeg stops
with `Encoder requires nv12 pixel format`.

Fedora's ffmpeg does not know `P010` on V4L2 devices, so 10-bit output is
not available from the command line. Use the HandBrake build for Main 10.

### HandBrake with T2 AVE support

The `t2-ave` branch of [our HandBrake fork](https://github.com/deqrocks/HandBrake)
adds the encoder as **H.265 (Apple T2 AVE)** and teaches its bundled ffmpeg
the 10-bit input format. Choose the `main10` profile for 10-bit output.

Install our HandBrake fork with

```bash
sudo dnf install git gcc gcc-c++ make autoconf automake libtool pkgconf-pkg-config meson ninja-build nasm cmake cargo cargo-c patch tar python3 bzip2-devel xz-devel zlib-devel numactl-devel gtk4-devel glib2-devel libxml2-devel lame-devel opus-devel speex-devel libvpx-devel libass-devel libogg-devel libvorbis-devel libtheora-devel x264-devel jansson-devel libjpeg-turbo-devel gettext-devel desktop-file-utils
git clone --branch t2-ave https://github.com/deqrocks/HandBrake.git
cd HandBrake && ./configure --launch-jobs="$(nproc)" --launch
sudo make -C build install
```

## Sidecar encoder

The T2 offers this encoder to macOS for Sidecar. It only exists while the
`kait2en-t2-remote` service holds its connection to the T2. The T2 then shows
the virtual USB device "SideCar AVE" (`05ac:9312`) and the `t2bce_sidecar`
module turns it into the `t2-sidecar` encoder.

- H.264 (High profile) and HEVC (Main profile) from 8-bit `NV12` input.
- About 85 frames per second at 1920x1080 and 50 at 2560x1600.
- About 20 ms from handing over a frame to receiving it encoded.
- Double buffered, with scatter-gather DMA.

```bash
ffmpeg -i input.mp4 -pix_fmt nv12 -c:v h264_v4l2m2m -b:v 10M -c:a copy output.mp4
```

ffmpeg uses the first `/dev/video` node that can encode the requested format.
H.264 always lands on `t2-sidecar` because the AVE encoder has no H.264. With
`hevc_v4l2m2m` ffmpeg can pick either encoder, so Sidecar streaming uses H.264.
H.264 also has lower latency at a higher bitrate.
