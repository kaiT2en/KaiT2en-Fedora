# Licensing

Per-file `SPDX-License-Identifier` headers are authoritative; this table is the
overview. Already-released versions stay under the license they were published
under, a license choice only applies going forward.

## GPL-2.0 (kernel modules)

Kernel modules under `modules/` and `t2-services/t2-ave/kernel/` stay GPL-2.0 as the
kernel requires. This includes `t2bce_*`, `t2ave` (in the BCE stack), `t2sep`,
`t2smc`, `t2smp` and the others.

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

## Attribution (GPL-3.0 section 7b)

The GPL-3.0-or-later components carry an additional term under section 7(b):
their author attribution must be preserved when the work is conveyed, and
reproduced in the Appropriate Legal Notices displayed by works that contain it.
Copyright of that userspace is held by André Eikmeyer
<andre.eikmeyer@kait2en.org>, and by 4f1sh3r <git@syn-flut.de> for
`t2-fan-control` and `t2-smc-control`. This is a reasonable attribution
requirement and adds no restriction beyond section 7(b).

## Other owners

- Vendored third-party code (e.g. `t2-services/t2-journal/libs/macos-unifiedlogs/`,
  `third-party/hex2hcd/`) keeps its upstream license.
