# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
MODULE := kait2en-t2-touchid

.PHONY: all install clean
all: $(MODULE).pp

$(MODULE).pp: $(MODULE).te $(MODULE).fc
	checkmodule -M -m -o $(MODULE).mod $(MODULE).te
	semodule_package -o $@ -m $(MODULE).mod -f $(MODULE).fc

install: $(MODULE).pp
	semodule -i $(MODULE).pp

clean:
	rm -f $(MODULE).mod $(MODULE).pp
