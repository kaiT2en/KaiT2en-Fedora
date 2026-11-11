# SPDX-License-Identifier: GPL-2.0-only
#
# Wrapper for the out-of-tree build, the module list is in Kbuild. DKMS passes
# KERNELRELEASE for the kernel it builds for.

KVERSION ?= $(if $(KERNELRELEASE),$(KERNELRELEASE),$(shell uname -r))
KDIR ?= /lib/modules/$(KVERSION)/build

all:
	$(MAKE) -C $(KDIR) M=$(CURDIR) modules

clean:
	$(MAKE) -C $(KDIR) M=$(CURDIR) clean

.PHONY: all clean
