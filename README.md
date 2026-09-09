<p align="center">
  <img src="assets/kait2en-fedora.jpeg" alt="KAIT2EN logo" width="720">
</p>

# KAIT2EN Fedora

KAIT2EN brings cutting edge T2 Mac support to stock Fedora using DKMS modules.
You will receive kernel updates directly from Fedora and the latest T2 modules from us.

[Docs](https://kait2en.org/documentation.html) |
[Blog](https://kait2en.org/blog.html) |
[Community](#community) |
[Contributing](#contributing)

## Install

Follow the [installation guide](https://kait2en.org/documentation.html#installation).

## HandBrake with T2 AVE

```bash
sudo dnf install git gcc gcc-c++ make autoconf automake libtool pkgconf-pkg-config meson ninja-build nasm cmake cargo cargo-c patch tar python3 bzip2-devel xz-devel zlib-devel numactl-devel gtk4-devel glib2-devel libxml2-devel lame-devel opus-devel speex-devel libvpx-devel libass-devel libogg-devel libvorbis-devel libtheora-devel x264-devel jansson-devel libjpeg-turbo-devel gettext-devel desktop-file-utils
git clone --branch t2-ave https://github.com/deqrocks/HandBrake.git
cd HandBrake && ./configure --launch-jobs="$(nproc)" --launch
sudo make -C build install
```

## Community

Join the KAIT2EN community on [Discord](https://discord.gg/AGfjRk4ydj) or on
[Matrix](https://matrix.to/#/%23kait2en:matrix.org).

## Contributing

Contributions are welcome, especially when they move KAIT2EN fixes closer to
clean upstream Linux support.

Please keep changes and PR descriptions focused.
**We will reject PRs and Issues when we notice we are talking to AI!**
At least talk to us in person.
We are not interested in workarounds.
We are not interested in major AI refactories.
There is a distinct difference between making broken things work and fixing things.

## License

KAIT2EN-owned scripts, howto documents, project text and helper code are MIT
licensed.

Kernel modules, apps and third-party tools may include code with different
origins. Those components keep their own licenses in their directories.
