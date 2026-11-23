# SPDX-License-Identifier: GPL-2.0-only
# Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
obj-m += t2_precision_trackpad.o

KDIR ?= /lib/modules/$(shell uname -r)/build
KVER ?= $(shell uname -r)
MODDEST ?= /lib/modules/$(KVER)/updates

all:
	$(MAKE) -C $(KDIR) M=$(CURDIR) modules

clean:
	$(MAKE) -C $(KDIR) M=$(CURDIR) clean

install: all
	install -d -o root -g root -m 0755 $(MODDEST)
	install -o root -g root -m 0644 t2_precision_trackpad.ko $(MODDEST)/
	depmod -a $(KVER)

.PHONY: all clean install
