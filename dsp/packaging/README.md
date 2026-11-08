# Independent DSP package

`t2-dsp` packages host-side audio processing, not a bridgeOS service or a
kernel module. RPM and Debian builds contain all 13 model profiles and require
neither T2 hardware nor network access. No compiled plugins are bundled.

## Build

From the repository root, create a source archive:

```sh
bash dsp/packaging/source-archive.sh /tmp/t2-dsp-0.1.0.tar.gz
```

The archive contains `dsp/`, `packaging/lifecycle/`, `LICENSE`, `LICENSING.md` and `LICENSES/` under
`t2-dsp-0.1.0/`. Generated files and Python caches are excluded.

For RPM, put the archive in the builder's `SOURCES/` directory and run:

```sh
rpmbuild -ba dsp/packaging/rpm/t2-dsp.spec
sudo dnf install ./t2-dsp-0.1.0-1*.noarch.rpm
```

For Debian/Ubuntu, unpack the archive, copy `dsp/packaging/debian/` to
`debian/` in the extracted root, and put a copy of the archive beside that
directory named `t2-dsp_0.1.0.orig.tar.gz`. Inside the source tree:

```sh
dpkg-buildpackage -us -uc
sudo apt install ../t2-dsp_0.1.0-1_all.deb
```

Once a repository publishes the package, use `dnf install t2-dsp` or
`apt install t2-dsp`. These recipes do not publish a repository.

Runtime dependencies are PipeWire with filter-chain/LV2 support, WirePlumber
0.5.8 or newer, the Bankstown and LSP LV2 plugins and udev:

| Fedora | Debian/Ubuntu |
|---|---|
| `pipewire` | `pipewire` |
| `pipewire-pulseaudio` | `pipewire-pulse` |
| `wireplumber >= 0.5.8` | `wireplumber (>= 0.5.8)` |
| `pipewire-module-filter-chain-lv2` | `libspa-0.2-modules` |
| `lv2-bankstown` | `bankstown-lv2` |
| `lsp-plugins-lv2` | `lsp-plugins-lv2` |
| `systemd-udev` | `udev` |

The driver and the AppleT2x2/x4/x6 UCM profiles come from separate packages.
The UCM source is `modules/t2bce_audio-alsa-ucm-conf/ucm2/` in the
alsa-ucm-conf layout. Add the distribution-specific dependencies once those
are packaged. WirePlumber must support UCM split PCMs.

## Validation

`make -C dsp test` runs hardware-free build, routing and migration tests.
With PipeWire and the runtime plugins installed,
`python3 dsp/tests/smoke-pipewire.py` loads all 24 graphs (13 speaker,
11 microphone) in a private PipeWire daemon and checks their 48 nodes. It
opens no hardware and stays outside the user's PipeWire session.

RPM builds, Debian 13 builds and an APT install with dependencies are tested.
T2 playback, recording and headphone switching need a hardware test after
reboot.

## Activation, migration and removal

Post-install scripts leave user audio, live card names, GNOME, dconf and PAM
as they are. Reboot after installation or removal, so udev sets the model ID
before ALSA/PipeWire opens the card.

`package-actions configure` writes
`/etc/pipewire/pipewire.conf.d/51-t2-dsp.conf`, a copy of
`/usr/share/t2-dsp/pipewire/<profile>.conf` selected from the DMI product
name of the installing machine. The package file list leaves it out. Image
builds on other hardware therefore get the build host's graph, or an error
for unlisted models. Removal deletes the file when it is unchanged.

Former generated `/etc` DSP and quantum fragments and matching profile data
under `/usr/share/kait2en/audio-dsp/` are backed up to
`/var/lib/kait2en/migration/t2-dsp/` and retired. Ownership receipts live in
`/var/lib/kait2en/ownership/t2-dsp.json`. Removal deletes verified, unchanged
backups and the receipt. Modified or unrecognized files are preserved and
reported.

Lifecycle failures are printed in a final summary and logged to
`/var/log/t2-dsp-install.log`. They leave the package transaction intact.
Build and staging failures fail the package build. The lifecycle engine
needs Python 3, its source is `packaging/lifecycle/`.

## Licenses

The package includes the full GPL-3.0-or-later text, `LICENSE`, `LICENSING.md`
and each profile's attribution notice. The section 7(b) attribution term
continues to apply. External plugins retain their own package licenses.
