# t2smc - Out-of-tree kernel module for T2 Mac SMC (fan + battery charge limit)
#
# Build requirements:
#   Kernel headers matching your running kernel (e.g. kernel-devel)
#
# Build:
#   make
#
# Install:
#   sudo make install
#
# Uninstall:
#   sudo make uninstall

obj-m := t2smc.o

KERNEL_SRC ?= /lib/modules/$(shell uname -r)/build
KERNEL_RELEASE ?= $(shell uname -r)
INSTALL_MOD_DIR ?= extra
MODDEST := /lib/modules/$(KERNEL_RELEASE)/$(INSTALL_MOD_DIR)
PWD := $(shell pwd)

all: modules

modules:
	$(MAKE) -C $(KERNEL_SRC) M=$(PWD) modules

install:
	@test -f t2smc.ko || { echo "t2smc.ko not found; run make first"; exit 1; }
	install -d -o root -g root -m 0755 $(MODDEST)
	install -o root -g root -m 0644 t2smc.ko $(MODDEST)/t2smc.ko
	depmod -a $(KERNEL_RELEASE)

uninstall:
	rm -f $(MODDEST)/t2smc.ko
	depmod -a $(KERNEL_RELEASE)

clean:
	$(MAKE) -C $(KERNEL_SRC) M=$(PWD) clean
	find $(PWD) -type f \( -name '*.o' -o -name '*.ko' -o -name '*.mod' -o -name '*.mod.c' -o -name '.*.cmd' -o -name 'Module.symvers' -o -name 'modules.order' \) -delete

.PHONY: all modules install uninstall clean
