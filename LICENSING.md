# Licensing

Per-file `SPDX-License-Identifier` headers are authoritative; this table is the
overview. Already-released versions stay under the license they were published
under, a license choice only applies going forward.

## GPL-2.0 (kernel modules)

Everything under `modules/` is a Linux kernel module and stays GPL-2.0 as the
kernel requires. This includes `t2bce_*`, `t2ave` (in the BCE stack), `t2sep`,
`t2smc`, `t2smp` and the others.

## GPL-3.0-or-later

KAIT2EN userspace authored solely by the project owner:

- `apps/t2-cpu-control`, `apps/t2-dgpu-control`, `apps/t2-hybrid-gpu-control`
- `apps/t2-journal`, `apps/t2-kernel-builder`
- `apps/t2-power-explorer`, `apps/t2-power-tune`
- `apps/t2-touchid`, `apps/t2-aks`, `apps/t2-touchid-probe`
- `apps/t2-fan-control`, `apps/t2-smc-control`

Full text: `LICENSES/GPL-3.0-or-later.txt`.

## Attribution (GPL-3.0 section 7b)

The GPL-3.0-or-later components carry an additional term under section 7(b):
their author attribution must be preserved when the work is conveyed, and
reproduced in the Appropriate Legal Notices displayed by works that contain it.
Copyright of that userspace is held by André Eikmeyer
<andre.eikmeyer@kait2en.org>, and by 4f1sh3r <git@syn-flut.de> for
`t2-fan-control` and `t2-smc-control`. This is a reasonable attribution
requirement and adds no restriction beyond section 7(b).

## MIT (permissive)

Kept permissive on purpose so the Touch ID transport and protocol can go into
libfprint (LGPL) later:

- `protocols/t2-bridgexpc`, `protocols/t2-biometrickit`

The root scripts, howto docs and project text stay MIT as well (root `LICENSE`).

## Other owners

- `apps/react-drm` carries its own license.
- Vendored third-party code (e.g. `apps/t2-journal/vendor/`) keeps its upstream
  license.
