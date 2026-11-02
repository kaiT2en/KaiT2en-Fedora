# KAIT2EN audio DSP profile imacpro1_1 (iMacPro1,1)

Speaker DSP for the internal speakers of the iMacPro1,1: `graph.json` is the
PipeWire filter-chain graph. There are no crosstalk cancellation FIRs
because the macOS speaker graph of this model has no XTC box. Installed by
`scripts/fedora/install-dsp.sh`.

Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>

Licensed under the GNU General Public License, version 3 or later
(GPL-3.0-or-later, full text in `LICENSES/GPL-3.0-or-later.txt`), with the
attribution term of `LICENSING.md`: keep the copyright notices when you
redistribute or derive from these files.
