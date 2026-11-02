# KAIT2EN audio DSP profile imac20_1 (iMac20,1)

Speaker DSP for the internal speakers of the iMac20,1: `graph.json` is the
PipeWire filter-chain graph. There are no crosstalk cancellation FIRs
because macOS runs no XTC on this model. The XTC box is commented out in its
speaker graph. Installed by `scripts/fedora/install-dsp.sh`.

Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>

Licensed under the GNU General Public License, version 3 or later
(GPL-3.0-or-later, full text in `LICENSES/GPL-3.0-or-later.txt`), with the
attribution term of `LICENSING.md`: keep the copyright notices when you
redistribute or derive from these files.
