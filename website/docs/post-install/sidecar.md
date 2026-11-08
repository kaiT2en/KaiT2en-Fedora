# Streaming using Sidecar

With the T2's Sidecar encoder, the desktop can be streamed to an Android tablet,
iPad, a phone or another computer. [Sunshine](https://github.com/LizardByte/Sunshine)
runs on the Mac and encodes with the T2, and
[Moonlight](https://moonlight-stream.org) shows the stream on the other device
and sends touch, mouse and keyboard input back. The T2 encodes 1920x1080 at
about 85 frames per second with about 20 ms latency and leaves the CPU free.

The encoder is described in
[Hardware video encoding](hardware-video-encoding.md#sidecar-encoder).

## Sunshine with V4L2 support

V4L2 encoder support for Sunshine is in
[pull request #5717](https://github.com/LizardByte/Sunshine/pull/5717), still
under review. The `t2-sidecar` branch of
[our Sunshine fork](https://github.com/deqrocks/Sunshine) carries it unchanged
until it is merged. The GPU converts every captured frame directly into the
encoder's input buffers.

Install our Sunshine fork with

```bash
sudo dnf install git cmake ninja-build gcc gcc-c++ glslc npm python3-jinja2 desktop-file-utils libappstream-glib appstream boost-devel boost-static libcap-devel libcurl-devel libdrm-devel libevdev-devel libva-devel libX11-devel libxcb-devel libXcursor-devel libXfixes-devel libXi-devel libXinerama-devel libXrandr-devel libXtst-devel mesa-libGL-devel mesa-libgbm-devel miniupnpc-devel numactl-devel openssl-devel opus-devel pipewire-devel pulseaudio-libs-devel qt6-qtbase-devel qt6-qtsvg-devel vulkan-loader-devel
git clone --branch t2-sidecar --recurse-submodules --shallow-submodules https://github.com/deqrocks/Sunshine.git
cd Sunshine
cmake -B build -G Ninja -DCMAKE_BUILD_TYPE=Release -DBUILD_DOCS=OFF -DBUILD_TESTS=OFF -DSUNSHINE_ENABLE_CUDA=OFF -DSUNSHINE_ASSETS_DIR=share/sunshine
ninja -C build
sudo cmake --install build
```

## Configuration

Put this into `~/.config/sunshine/sunshine.conf`:

```ini
encoder = v4l2m2m
capture = portal
hevc_mode = 1
av1_mode = 1
system_tray = disabled
```

- `encoder = v4l2m2m` selects the V4L2 encoder.
- `hevc_mode = 1` and `av1_mode = 1` keep the stream on H.264. With HEVC,
  ffmpeg may pick the AVE encoder instead of `t2-sidecar`.
- `capture = portal` records the screen through GNOME's screen sharing.
- `system_tray = disabled` is needed on GNOME. Without a tray, Sunshine
  otherwise ends right after starting.

Start `sunshine` from a terminal. On the first start GNOME asks which screen
to share. Pick the built-in display. Sunshine remembers the choice and restarts
itself. Then open <https://localhost:47990>, create a user name and password,
and pair Moonlight with the PIN it shows.

## Extended display

To use the tablet as a second screen, switch on the Sidecar display:

```bash
t2-sidecar on
```

GNOME then shows an additional monitor next to the built-in display. Arrange it,
set its resolution or switch to mirroring in Settings > Displays like any other
monitor. Pick the same resolution as in Moonlight, otherwise Sunshine scales
every frame and the picture gets less sharp. The next `t2-sidecar on` starts
with the resolution picked last.

With Sunshine installed to `/usr/local/bin/sunshine` as described above,
`t2-sidecar on` also starts Sunshine, and GNOME asks which screen to share. Pick
the new monitor. Sunshine runs only while the Sidecar display is on, so GNOME's
screen sharing indicator is not shown otherwise. Do not start `sunshine` from a
terminal at the same time.

`t2-sidecar off` stops Sunshine and removes the monitor. Windows on it move back
to the built-in display. Locking the screen and logging out switch it off as
well.

Every `t2-sidecar on` creates a new virtual monitor, and GNOME does not apply a
saved screen choice to it. That is why the dialog comes up each time.

## Moonlight settings

- 1080p at 60 frames per second. 2560x1600 reached 50 to 60 frames per
  second in our tests over Wi-Fi 6 with a Snapdragon Android tablet.
- Video codec H.264 or automatic.
- Sound plays on the tablet while streaming. To keep it on the Mac, enable
  playing audio on the PC in Moonlight's settings. To never stream sound from
  any device, add `stream_audio = disabled` to `sunshine.conf`.

## Limitations

Sunshine logs "Encoder did not produce IDR frame when requested" while
streaming. The encoder returns frames asynchronously and Sunshine checks the
wrong packet. Requested keyframes arrive on the requested frame.
