# Hardware video encoding

The T2 contains Apple's video encoder, the AVE. KAIT2EN exposes it to Linux
as a standard V4L2 memory-to-memory encoder, so ffmpeg, HandBrake and anything
else that speaks V4L2 can hand it raw frames and receive HEVC. The Intel and
AMD GPUs keep doing the decoding; the T2 only encodes.

- HEVC (H.265) only. There is no H.264 output.
- Main profile from 8-bit `NV12` input, Main 10 from `P010` input.
- Frame sizes from 128x128 up to 4096x2304.
- One encode at a time. A second session is refused with `EBUSY` until the
  first has finished.

## ffmpeg

Fedora's `ffmpeg-free` already ships the V4L2 encoder wrapper. The encoder
expects `NV12` frames, so tell ffmpeg to convert:

```bash
ffmpeg -i input.mp4 -pix_fmt nv12 -c:v hevc_v4l2m2m -b:v 8M -c:a copy output.mp4
```

`-b:v` sets the target bitrate and `-g` the keyframe interval; `-profile:v`
and `-level` are passed through as well. Without `-pix_fmt nv12` ffmpeg stops
with `Encoder requires nv12 pixel format`.

Fedora's ffmpeg does not know `P010` on V4L2 devices, so 10-bit output is
not available from the command line. Use the HandBrake build for Main 10.

## HandBrake with T2 AVE Support

The `t2-ave` branch of [our HandBrake fork](https://github.com/deqrocks/HandBrake)
adds the encoder as **H.265 (Apple T2 AVE)** and teaches its bundled ffmpeg
the 10-bit input format. Choose the `main10` profile for 10-bit output. 

Install our Handbrake fork with

```bash
sudo dnf install git gcc gcc-c++ make autoconf automake libtool pkgconf-pkg-config meson ninja-build nasm cmake cargo cargo-c patch tar python3 bzip2-devel xz-devel zlib-devel numactl-devel gtk4-devel glib2-devel libxml2-devel lame-devel opus-devel speex-devel libvpx-devel libass-devel libogg-devel libvorbis-devel libtheora-devel x264-devel jansson-devel libjpeg-turbo-devel gettext-devel desktop-file-utils
git clone --branch t2-ave https://github.com/deqrocks/HandBrake.git
cd HandBrake && ./configure --launch-jobs="$(nproc)" --launch
sudo make -C build install
```