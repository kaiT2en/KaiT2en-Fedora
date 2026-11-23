# Licensing

Per-file `SPDX-License-Identifier` headers are authoritative; this table is the
overview. Already-released versions stay under the license they were published
under, a license choice only applies going forward.

## GPL-2.0 (kernel modules)

Kernel modules under `modules/` and `t2-services/t2-ave/kernel/` stay GPL-2.0 as the
kernel requires, either GPL-2.0-only or GPL-2.0-or-later as each file header
states. This includes `apple-t2-bridge`, `t2-ave`, `t2-sidecar`, `t2sep`,
`t2smc`, `t2smp` and the others. Several of them are derived from upstream
Linux drivers and keep the copyright notices of their original authors.

Full text: `LICENSES/GPL-2.0.txt`.

## GPL-3.0-or-later

Everything that is not a kernel module and not vendored third-party code:

- all programs and tools below `apps/`
- userspace programs and integration below `t2-services/`
- `t2-services/shared/protocols/t2-bridgexpc`, `t2-services/t2-touchid/protocols/t2-biometrickit`
- host-side DSP integration and profiles below `dsp/` (each profile
  folder carries a README.md with the notice; the installer copies it next to
  the installed files)
- the root scripts, howto docs and project text (root `LICENSE`)

Full text: `LICENSES/GPL-3.0-or-later.txt`.

## Copyright holders

Copyright of the GPL-3.0-or-later userspace is held by its authors. Most of it
is written by André Eikmeyer <andre.eikmeyer@kait2en.org>. 4f1sh3r
<git@syn-flut.de> holds the copyright of his contributions, in particular large
parts of `auto-installer/`, `scripts/macos/`, `scripts/tests/`, `scripts/ci/`
and `scripts/fedora/`, and his contributions to `t2-fan-control` and
`t2-smc-control`. Smaller contributions by other authors stay theirs; `AUTHORS`
lists them and the git history records every one.

## Additional terms (GPL-3.0 section 7)

The GPL-3.0-or-later components carry the following additional terms. Both are
of the kinds section 7 permits and add no restriction beyond it.

### Attribution, section 7(b)

The author attribution must be preserved when the work is conveyed, and
reproduced in the Appropriate Legal Notices displayed by works that contain it.

### Origin, section 7(c)

For the material whose copyright is held by André Eikmeyer:
the origin of that material must not be misrepresented, and modified versions
must be marked in reasonable ways as different from the original version. A
modified version must not be presented as KAIT2EN or as the original work, and
its Appropriate Legal Notices and the README or equivalent documentation
conveyed with it must state that it is based on KAIT2EN
(https://github.com/kaiT2en/KaiT2en-Fedora).

### Trademarks, section 7(e)

No rights are granted under trademark law for the name "KAIT2EN", the KAIT2EN
logo or the other brand assets listed below. How the name may be used is
described in `TRADEMARK.md`.

## KAIT2EN name, logo and brand assets

The KAIT2EN logo, mark and brand artwork are not covered by the GPL or any
other license in this repository. They are © 2026 André Eikmeyer, all rights
reserved, and identify the KAIT2EN project. They are shipped so that KAIT2EN
itself can display them; a fork or other derived work must remove or replace
them. The files are:

- `assets/kaiT2en-logo.png`, `assets/kaiT2en-logo-tr.png`, `assets/kaiT2en-logo-tr.svg`
- `assets/kait2en-app-logo.png`, `assets/kait2en-app-logo-light.png`
- `assets/gdm/kait2en-gdm-logo.png`
- `assets/plymouth/kait2en/animation-0001.png`, `assets/plymouth/kait2en/watermark.png`
- `assets/kanji.ans`, `assets/kanji.txt`
- `website/static/img/og-image.png`, `website/static/img/favicon-32.png`,
  `website/static/img/favicon-192.png`, `website/static/img/apple-touch-icon.png`

This applies to versions published on or after 2026-11-23. `TRADEMARK.md` has
stated since 2026-09-11 that the code licenses do not grant the name or logo.

## Other owners

- Vendored third-party code (e.g. `t2-services/t2-journal/libs/macos-unifiedlogs/`,
  `third-party/hex2hcd/`) keeps its upstream license.
- `third-party/linux-apfs-rw/` is GPL-2.0-only. Its `lzfse/` files are
  BSD-3-Clause, copyright Apple Inc., and `libzbitmap.*` is
  GPL-2.0-or-later OR MIT. The notices stay in the file headers.
- `apps/t2-touchbar/src/display.rs` derives its low-level DRM setup from
  tiny-dfr. Its MIT notice is preserved in
  `apps/t2-touchbar/THIRD-PARTY-NOTICES.md`; the combined KAIT2EN program is
  distributed under GPL-3.0-or-later.
- `apps/t2-airshare/proto/` holds the Nearby protobuf definitions, Apache-2.0
  (Google LLC) and BSD-style (The Chromium Authors) with their license texts
  next to them. Parts of `apps/t2-airshare/src/secure.rs`, `link.rs`,
  `receive.rs`, `send.rs` and `discover.rs` are adapted from rquickshare
  (GPL-3.0). The
  notices are in `apps/t2-airshare/THIRD-PARTY-NOTICES.md`, the combined
  KAIT2EN program is distributed under GPL-3.0-or-later.
- `scripts/fedora/other/download-bootcamp-drivers.py` is based on Brigadier by
  Timothy Sutton and is MIT, see `download-bootcamp-drivers.LICENSE.md` next
  to it.
- The password dialog assets in `assets/plymouth/kait2en/` (`entry.png`,
  `lock.png`, `bullet.png`, `keyboard.png`, `capslock.png`,
  `keymap-render.png`) are adapted from Fedora's Spinner Plymouth theme and are
  GPL-2.0-or-later, see `CREDITS.md` there.
- `assets/fonts/` holds JetBrains Mono, copyright The JetBrains Mono Project
  Authors, under the SIL Open Font License 1.1 (`assets/fonts/OFL.txt`).
- The wallpapers in `assets/vornehm/backgrounds/` are from the BLACK
  collection by Jean-Marc Denis (https://jmd.im/black), © Jean-Marc Denis.
  They are not covered by the KaiT2en licenses. No redistribution license is
  stated, they are shipped downscaled with credit in good faith and removed
  on the artist's request. Details in `assets/vornehm/backgrounds/CREDITS.md`.

## Trademarks

Touch ID and  Airdrop are a trademark of Apple Inc. This project is not affiliated
with Apple. Apple names are used only to describe the hardware and features this
project supports.
