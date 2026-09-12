# Licensing

Per-file `SPDX-License-Identifier` headers are authoritative; this table is the
overview. Already-released versions stay under the license they were published
under, a license choice only applies going forward.

## GPL-2.0 (kernel modules)

Everything under `modules/` is a Linux kernel module and stays GPL-2.0 as the
kernel requires. This includes `t2bce_*`, `t2ave` (in the BCE stack), `t2sep`,
`t2smc`, `t2smp` and the others.

## KAIT2EN License 1.0 (userspace, not open source)

Everything userspace authored by the project owner carries the KAIT2EN License
1.0 (`LICENSES/KAIT2EN-1.0.txt`, SPDX `LicenseRef-KAIT2EN-1.0`): use with
KAIT2EN on your own Apple T2 Mac, modification for your own machines,
redistribution only unmodified as part of KAIT2EN, no use in other projects
without written permission, attribution required, and an explicit exclusion of
David Heinemeier Hansson, the Omacom Foundation and the Omakub and Omarchy
projects with their forks. Versions released before 2026-09-12 stay under the
license they were published under.

- `apps/t2-cpu-control`, `apps/t2-dgpu-control`, `apps/t2-gpu-switch`,
  `apps/t2-hybrid-gpu-control`
- `apps/t2-journal`, `apps/t2-kernel-builder`
- `apps/t2-power-explorer`, `apps/t2-power-tune`
- `apps/t2-touchid`, `apps/t2-aks`, `apps/t2-touchid-probe`
- the audio DSP profiles below `modules/t2bce_audio-dsp/profiles/` (each
  folder carries a README.md with the notice; the installer copies it next to
  the installed files)

Copyright of that userspace is held by André Eikmeyer
<andre.eikmeyer@kait2en.org>.

`apps/t2-fan-control` and `apps/t2-smc-control` are co-owned by 4f1sh3r
<git@syn-flut.de> and stay GPL-3.0-or-later with the section 7(b) attribution
term (`LICENSES/GPL-3.0-or-later.txt`) until that contributor agrees to the
change.

## MIT (permissive)

Kept permissive on purpose so the Touch ID transport and protocol can go into
libfprint (LGPL) later:

- `protocols/t2-bridgexpc`, `protocols/t2-biometrickit`

The root scripts, howto docs and project text stay MIT as well (root `LICENSE`).

## Other owners

- `apps/react-drm` carries its own license.
- Vendored third-party code (e.g. `apps/t2-journal/vendor/`) keeps its upstream
  license.
